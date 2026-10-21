"""比高形状の比較を生成し、データ・ブラウザー・山の近傍密度を検証する。"""
import json, os, sqlite3, struct, subprocess, zlib
from pathlib import Path
import numpy as np
from compare_shared_allocation import main, ROOT
from verify_shared_allocation import verify


def density(mode):
    result={}
    with sqlite3.connect(ROOT/f'build/terrain-shared-allocation-{mode}.sqlite3') as db:
        manifest=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
        for area in manifest['reliefRegions']:
            keys=area['levels'][0]['keys'];ox=min(v[1] for v in keys);oy=min(v[2] for v in keys);points=set()
            for l,x,y in keys:
                raw=zlib.decompress(db.execute('SELECT geometry FROM tiles WHERE level=? AND x=? AND y=?',(l,x,y)).fetchone()[0]);nv=struct.unpack_from('<I',raw,8)[0]
                p=np.frombuffer(raw,'<f4',nv*3,16).reshape(-1,3).astype(float);p[:,:2]+=[(x-ox)*8000,(y-oy)*8000]
                on_grid=np.max(np.abs(p[:,:2]/31.25-np.rint(p[:,:2]/31.25)),axis=1)<1e-6
                points.update(map(tuple,p[on_grid,:2]))
            p=np.asarray(list(points));r2=((p-manifest['sharedAllocation'].get('widthMeters',32000)/2)**2).sum(axis=1)
            result[area['id']]=dict(sourceVertices=area['levels'][0]['priorityMetrics']['sourceVertices'],withinRadius={str(radius):int((r2<=(radius*1000)**2).sum()) for radius in [2,5,10,20]})
    return result


if __name__=='__main__':
    binary=ROOT/'build/terrain-rust-incremental-target/release/terrain-adaptive-tin.exe'
    main(192000,8192,True,128000.,True,str(binary),shape_aware=True)
    result=verify('shape128');assert result['totalSource']==192000
    node=Path.home()/'.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node.exe'
    env=os.environ.copy();env['NODE_PATH']=str(node.parents[1]/'node_modules');env['ALLOCATION_MODES']='shape128'
    subprocess.run([str(node),'tools/terrain/test_shared_allocation_view.cjs'],cwd=ROOT,env=env,check=True)
    report=dict(complete=True,baseline=density('terrain128'),shape=density('shape128'),verification=result,sqliteBytes=(ROOT/'build/terrain-shared-allocation-shape128.sqlite3').stat().st_size)
    (ROOT/'build/terrain-shape-allocation-report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
    print('形状比高比較の生成・検証完了',report,flush=True)
