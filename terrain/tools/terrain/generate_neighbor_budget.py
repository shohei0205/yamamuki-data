"""保存セル単位の均等予算と、品質達成セルからの近傍譲渡を比較する。"""
import argparse,json,sqlite3,subprocess
from contextlib import closing
from pathlib import Path
import numpy as np
from generate_shared_quality_budgets import cached_prepare
from refine_shared_quality import ROOT,compare,save,angles
from compare_shared_allocation import residual
from uncut_mesh_topology import tile_topology,update_metrics
from verify_shared_allocation import verify


def run(sharing,base,offset=0):
    mode=('local-share128' if sharing else 'local-fixed128')+('-shift' if offset else '');path=ROOT/f'build/terrain-shared-allocation-{mode}.sqlite3'
    if path.exists():raise ValueError('既存の比較出力は上書きしません')
    binary=ROOT/'build/terrain-rust-local-budget-target/release/terrain-adaptive-tin.exe'
    compare.initialize();regions=[];meshes=[];reports={}
    with closing(sqlite3.connect(ROOT/'build/terrain-shared-allocation-landform128.sqlite3')) as source,closing(sqlite3.connect(ROOT/'build/terrain-regional-priority-levels-v5.sqlite3')) as template:
        m=json.loads(source.execute('select value from metadata where key="manifest"').fetchone()[0]);templates=json.loads(template.execute('select value from metadata where key="manifest"').fetchone()[0])['reliefRegions']
        for area in m['reliefRegions']:
            name=area['id'];r=cached_prepare(next(v for v in templates if v['id']==name),source);folder=r['folder'];output=ROOT/f'build/terrain-{mode}-{name}.json'
            command=[str(binary),'--input',str(folder/'height.png'),'--output',str(output),'--scale','.25','--offset',str(r['height_offset']),'--error-limits',str(folder/'balanced.png'),'--tile-pixels','256','--tile-offset',str(offset),'--tile-base',str(base),'--tile-radius','2','--max-vertices',str(base*256)]
            if not sharing:command.append('--tile-no-sharing')
            print('近傍配分の生成',mode,name,flush=True);subprocess.run(command,check=True)
            data=json.loads(output.read_text(encoding='utf-8'));p=np.asarray(data['vertices'],float);f=np.asarray(data['triangles'],np.uint32);allocation=data['tileAllocation']
            assert sum(allocation['counts'])==len(p) and sum(allocation['quotas'])==base*256
            assert all(c<=q<=b*2 for c,q,b in zip(allocation['counts'],allocation['quotas'],allocation['baseline']))
            delta=residual(p,f,r['grid']);v=delta[r['valid']];ratio=np.max(np.abs(v)/r['shape_limits'][r['valid']]);a=angles(p,f)
            mesh=dict(p=p,f=f,maximum=float(np.abs(v).max()),rmse=float(np.sqrt(np.mean(v*v))),weightedMaximum=float(ratio),score=float(ratio))
            reports[name]=dict(allocation=allocation,sourceVertices=len(p),minimumAngle=float(a.min()),under10=int((a<10).sum()),beforeUnder10=int((a<10).sum()),rmse=mesh['rmse'],maximum=mesh['maximum'])
            print('地域完了',name,'使用',len(p),'譲渡',allocation['transfers'],'RMSE',mesh['rmse'],flush=True)
            regions.append(r);meshes.append(mesh)
        save('balanced',regions,meshes,m,base*256*3,[],variant=mode)
    with closing(sqlite3.connect(path)) as db,db:
        m=json.loads(db.execute('select value from metadata where key="manifest"').fetchone()[0]);topology=[]
        for area,mesh in zip(m['reliefRegions'],meshes):
            keys=area['levels'][0]['keys'];ox=min(v[1] for v in keys);oy=min(v[2] for v in keys);records={tuple(v) for v in keys};world=mesh['p'].copy();world[:,:2]*=31.25;ranks=np.arange(len(world))/max(1,len(world)-1)
            for (x,y),body,count in tile_topology(world,mesh['f'],ranks=ranks):
                key=(3,ox+x,oy+y)
                if key in records:db.execute('insert or replace into topology values(?,?,?,?)',(*key,body))
            topology.append(dict(region=area['id'],source='direct-parent-tin',facesBefore=len(mesh['f']),facesAfter=len(mesh['f'])))
        a=m['sharedAllocation'];a['topologyReports']=topology;a['localAllocation']=dict(baseVerticesPerTile=base,originOffsetMeters=offset*31.25,tileWidthMeters=8000,maximumMultiplier=2,radiusTiles=2,sharing=sharing,donorAngleMinimum=10,donorErrorRatioMaximum=1,transferBatchMaximum=32,selection='normalized-error-over-one-plus-distance',scope='within-each-128km-prototype')
        a['meshQuality']=dict(regions=reports);a['generationComplete']=True;update_metrics(db,m)
    result=verify(mode);report=dict(mode=mode,sourceVertices=result['totalSource'],budget=base*256*3,tiles=result['tiles'],sharedEdges=result['sharedEdges'],regions=reports)
    (ROOT/f'build/terrain-{mode}-report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8');print('生成・保存検証完了',mode,flush=True)


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--base',type=int,default=64);p.add_argument('--offset',type=int,choices=[0,128],default=0);a=p.parse_args()
    for sharing in [False,True]:run(sharing,a.base,a.offset)
