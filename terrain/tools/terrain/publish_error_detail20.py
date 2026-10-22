"""検証済みの削減チェックポイントから比較用SQLiteを保存する。"""
import json,sqlite3
from contextlib import closing
import numpy as np
from generate_shared_quality_budgets import cached_prepare
from refine_shared_quality import ROOT,compare,save,angles
from compare_shared_allocation import residual
from uncut_mesh_topology import tile_topology,update_metrics
from verify_shared_allocation import verify
from generate_error_detail20 import MODE,progress
from mixed_storage_mesh import generate


def main():
    path=ROOT/f'build/terrain-shared-allocation-{MODE}.sqlite3'
    if path.exists():raise ValueError('既存の出力は上書きしません')
    compare.initialize();regions=[];meshes=[];reports={};base_total=0;used=0
    with closing(sqlite3.connect(ROOT/'build/terrain-shared-allocation-landform128.sqlite3')) as source,closing(sqlite3.connect(ROOT/'build/terrain-regional-priority-levels-v5.sqlite3')) as template:
        manifest=json.loads(source.execute('select value from metadata where key="manifest"').fetchone()[0]);templates=json.loads(template.execute('select value from metadata where key="manifest"').fetchone()[0])['reliefRegions']
        for area in manifest['reliefRegions']:
            name=area['id'];r=cached_prepare(next(v for v in templates if v['id']==name),source);regions.append(r);folder=ROOT/f'build/terrain-error20-{name}'
            with np.load(folder/'base-quality.npz') as data:base=len(data['p'])
            with np.load(folder/'pruned.npz') as data:p=data['p'];f=data['f'];ids=data['ids']
            pruning=json.loads((folder/'pruning.json').read_text());added=len(p)+pruning['removed']-base;base_total+=base;used+=added
            delta=residual(p,f,r['grid']);v=delta[r['valid']];maximum=float(np.abs(delta).max());a=angles(p,f);assert maximum<=25.+1e-5 and a.min()>=10.-1e-6
            mesh=dict(p=p,f=f,maximum=maximum,rmse=float(np.sqrt(np.mean(v*v))),weightedMaximum=maximum/25,score=maximum/25);meshes.append(mesh)
            reports[name]=dict(sourceVertices=len(p),initialVertices=int((ids<base).sum()),basicVerticesBeforePruning=base,addedBeforePruning=added,addedFromPool=int((ids>=base).sum()),pruning=pruning,maximum=maximum,rmse=mesh['rmse'],minimumAngle=float(a.min()),under10=int((a<10.-1e-6).sum()),beforeUnder10=0)
        budget=base_total//5;assert used<=budget;progress('検証済みメッシュを高速保存',base=base_total,budget=budget)
        save('balanced',regions,meshes,manifest,base_total+budget,[],variant=MODE,rebuild_topology=False)
    with closing(sqlite3.connect(path)) as db,db:
        db.execute('CREATE TABLE IF NOT EXISTS topology(level,x,y,data,PRIMARY KEY(level,x,y))');m=json.loads(db.execute('select value from metadata where key="manifest"').fetchone()[0]);m['levels'][0]['maxErrorMeters']=25;topology=[]
        for area,mesh in zip(m['reliefRegions'],meshes):
            progress('親TIN接続を保存',region=area['id']);keys=area['levels'][0]['keys'];ox=min(v[1] for v in keys);oy=min(v[2] for v in keys);records={tuple(v) for v in keys};world=mesh['p'].copy();world[:,:2]*=31.25;ranks=np.arange(len(world))/max(1,len(world)-1)
            for (x,y),body,count in tile_topology(world,mesh['f'],ranks=ranks):
                key=(3,ox+x,oy+y)
                if key in records:db.execute('insert into topology values(?,?,?,?)',(*key,body))
            topology.append(dict(region=area['id'],source='direct-parent-tin',facesBefore=len(mesh['f']),facesAfter=len(mesh['f'])))
        a=m['sharedAllocation'];a['topology']='uncut-halo-v3';a['topologyReports']=topology;a['errorDetailPolicy']=dict(baseErrorMeters=25,minimumAngleDegrees=10,additionalFraction=.2,basicVertices=base_total,additionalBudget=budget,additionalUsed=used,remaining=budget-used,densityRadiusMeters=4000,densityReferenceVertices=256,densityEvaluationGridMeters=500,priority='error*(1+0.5*slope+0.5*landform)/(1+nearbyVertices/256)',pruneMaximumSurfaceChangeMeters=2.5,mixedSizesShareParent=True)
        a['globalPool']=dict(totalBudget=base_total+budget,initialSurplus=budget,allocatedSurplus=used,unusedPool=budget-used);a['meshQuality']=dict(regions=reports);a['generationComplete']=True;update_metrics(db,m)
    verified=verify(MODE);generate();progress('生成完了',complete=True,verified=verified,regions=reports,base=base_total,extraBudget=budget)

if __name__=='__main__':main()
