"""頂点数を上限とし、誤差と角度を満たした時点で終了する比較データを生成する。"""
import argparse,json,sqlite3
from contextlib import closing
import numpy as np
from generate_shared_quality_budgets import cached_prepare,allocate,budget_parent,progress
from refine_shared_quality import ROOT,read_parent,compare,save
from capped_quality_mesh import build_to_quality
from uncut_mesh_topology import tile_topology,update_metrics
from retile_shared_mesh import retile


def main(total,resume_checkpoint=False):
    compare.initialize();compare.national.BINARY=str(ROOT/'build/terrain-rust-incremental-target/release/terrain-adaptive-tin.exe')
    mode=f'landform-quality128-b{total}-capped';path=ROOT/f'build/terrain-shared-allocation-{mode}.sqlite3'
    if path.exists():
        with closing(sqlite3.connect(path)) as existing:
            row=existing.execute('select value from metadata where key="manifest"').fetchone()
            if row and json.loads(row[0]).get('sharedAllocation',{}).get('generationComplete'):
                print('上限方式の既存データを保持',mode,flush=True);return
        raise ValueError('生成途中または別方式の比較データは上書きしません')
    with closing(sqlite3.connect(ROOT/'build/terrain-shared-allocation-landform128.sqlite3')) as db,closing(sqlite3.connect(ROOT/'build/terrain-regional-priority-levels-v5.sqlite3')) as template:
        manifest=json.loads(db.execute('select value from metadata where key="manifest"').fetchone()[0]);templates=json.loads(template.execute('select value from metadata where key="manifest"').fetchone()[0])['reliefRegions']
        regions=[];selected=[];weights=[]
        for area in manifest['reliefRegions']:
            r=cached_prepare(next(v for v in templates if v['id']==area['id']),db);p,rank=read_parent(db,area)
            chosen=budget_parent(p,rank,r['grid'],total);regions.append(r);selected.append(chosen);weights.append(area['levels'][0]['priorityMetrics']['sourceVertices'])
        caps=allocate(total,weights,[v[2] for v in selected]);meshes=[];reports={}
        for r,(p,rank,count,tol),cap in zip(regions,selected,caps):
            boundary=np.any((p[:,:2]==0)|(p[:,:2]==r['n']-1),axis=1);r['seed']=p[boundary,:2].astype(int).tolist()
            checkpoint=ROOT/f"build/terrain-capped-{total}-{r['area']['id']}-checkpoint.json"
            if resume_checkpoint and checkpoint.exists():r['seed']=json.loads(checkpoint.read_text(encoding='utf-8'))['seeds']
            print('上限方式の生成',r['area']['id'],cap,flush=True)
            mesh,report=build_to_quality(r,cap);report.update(boundaryToleranceMeters=tol,initialBoundaryVertices=count,initialSeedVertices=len(r['seed']))
            meshes.append(mesh);reports[r['area']['id']]=report;print('地域完了',r['area']['id'],report,flush=True)
        save('balanced',regions,meshes,manifest,total,[],variant=mode)
    with closing(sqlite3.connect(path)) as db,db:
        m=json.loads(db.execute('select value from metadata where key="manifest"').fetchone()[0]);topology=[]
        for area,mesh in zip(m['reliefRegions'],meshes):
            keys=area['levels'][0]['keys'];ox=min(v[1] for v in keys);oy=min(v[2] for v in keys);records={tuple(v) for v in keys};world=mesh['p'].copy();world[:,:2]*=31.25;ranks=np.arange(len(world))/max(1,len(world)-1)
            for (x,y),body,count in tile_topology(world,mesh['f'],ranks=ranks):
                key=(3,ox+x,oy+y)
                if key in records:db.execute('insert or replace into topology values(?,?,?,?)',(*key,body))
            topology.append(dict(region=area['id'],source='direct-parent-tin',facesBefore=len(mesh['f']),facesAfter=len(mesh['f'])))
        a=m['sharedAllocation'];a['topologyReports']=topology;a['meshQuality']=dict(budgetPolicy='cap-only',normalizedErrorTarget=1.,angleTarget=10.,regions=reports);update_metrics(db,m)
    width={12000:128000,24000:64000,48000:32000,96000:16000}.get(total,8000)
    if width!=8000:
        result=retile(mode,width);path.rename(path.with_name(path.stem+'-8km-before-retile.sqlite3'));__import__('pathlib').Path(result['pending']).rename(path)
    with closing(sqlite3.connect(path)) as db,db:
        m=json.loads(db.execute('select value from metadata where key="manifest"').fetchone()[0]);m['sharedAllocation']['generationComplete']=True;db.execute('update metadata set value=? where key="manifest"',(json.dumps(m),))
    result=dict(mode=mode,maximumVertices=total,sourceVertices=sum(len(v['p']) for v in meshes),regions=reports)
    (ROOT/f'build/terrain-capped-quality-{total}.json').write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf-8');print('完了',result,flush=True)


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--budget',type=int,choices=[12000,24000,48000,96000,192000,384000,768000],default=12000);p.add_argument('--resume-checkpoint',action='store_true');a=p.parse_args();main(a.budget,resume_checkpoint=a.resume_checkpoint)
