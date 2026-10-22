"""誤差制約の基本形状＋共通20%追加＋不要点削減を3地域で比較する。"""
import heapq,json,sqlite3,subprocess,time
from pathlib import Path
from contextlib import closing
import numpy as np
from PIL import Image
from generate_shared_quality_budgets import cached_prepare
from refine_shared_quality import ROOT,compare,save,angles
from error_budget_mesh import prune,base_mesh
from compare_shared_allocation import residual
from uncut_mesh_topology import tile_topology,update_metrics
from verify_shared_allocation import verify

MODE='error-detail20'
PROGRESS=ROOT/'build/terrain-error-detail20.progress.json'
def progress(stage,**values):
    row=dict(stage=stage,**values);PROGRESS.write_text(json.dumps(row,ensure_ascii=False),encoding='utf-8');print(stage,values,flush=True)

def checkpoint_folder(name,error=25.,perceptual=False,shape=False,minimum_additional_error=0.,scaled_cutoff=False,no_prune=False):
    return ROOT/f"build/terrain-error20{'-direct' if no_prune else '-cutoff' if scaled_cutoff else '-cut10' if minimum_additional_error else '-shape' if shape else '-perceptual' if perceptual else ''}{'' if error==25 else '-e'+format(error,'g')}-{name}"


def main(error=25.,perceptual=False,shape=False,minimum_additional_error=0.,scaled_cutoff=False,no_prune=False):
    if no_prune and (shape or minimum_additional_error or scaled_cutoff):raise ValueError('不要点削減なし版は平坦部抑制・追加誤差下限なしで生成します')
    if scaled_cutoff:minimum_additional_error=max(10.,error/4)
    if minimum_additional_error and shape:raise ValueError('追加誤差の下限は平坦部抑制方式だけに適用します')
    perceptual=perceptual or shape or bool(minimum_additional_error) or no_prune
    global MODE,PROGRESS
    MODE='error-detail20'+('-direct' if no_prune else '-cutoff' if scaled_cutoff else '-cut10' if minimum_additional_error else '-shape' if shape else '-perceptual' if perceptual else '')+('' if error==25 else '-e'+format(error,'g'))
    PROGRESS=ROOT/f'build/terrain-{MODE}.progress.json'

    path=ROOT/f'build/terrain-shared-allocation-{MODE}.sqlite3'
    if path.exists():raise ValueError('既存データは上書きしません')
    binary=ROOT/('build/terrain-rust-shape-target/release/terrain-adaptive-tin.exe' if shape or minimum_additional_error else 'build/terrain-rust-detail20-target/release/terrain-adaptive-tin.exe')
    compare.initialize();clients=[];logs=[];regions=[];initial=[];outputs=[];meshes=[];reports={}
    def request(client,command):
        client.stdin.write(command+'\n');client.stdin.flush();line=client.stdout.readline()
        if not line:raise RuntimeError('追加生成クライアントが終了: '+str(client.poll()))
        return json.loads(line)
    try:
        with closing(sqlite3.connect(ROOT/'build/terrain-shared-allocation-landform128.sqlite3')) as source,closing(sqlite3.connect(ROOT/'build/terrain-regional-priority-levels-v5.sqlite3')) as template:
            manifest=json.loads(source.execute('select value from metadata where key="manifest"').fetchone()[0]);templates=json.loads(template.execute('select value from metadata where key="manifest"').fetchone()[0])['reliefRegions'];ready=[]
            reuse_all=no_prune and all((checkpoint_folder(a['id'],error,perceptual=True)/'detail.json').exists() and (checkpoint_folder(a['id'],error,perceptual=True)/'base-quality.npz').exists() for a in manifest['reliefRegions'])
            for area in manifest['reliefRegions']:
                name=area['id'];r=cached_prepare(next(v for v in templates if v['id']==name),source);regions.append(r);folder=checkpoint_folder(name,error,perceptual,shape,minimum_additional_error,scaled_cutoff,no_prune)
                if perceptual:
                    import shutil
                    folder.mkdir(exist_ok=True)
                    base_source=(checkpoint_folder(name,error,perceptual=True) if reuse_all else checkpoint_folder(name,error))/'base-quality.npz'
                    if not (folder/'base-quality.npz').exists() and base_source.exists():shutil.copy2(base_source,folder/'base-quality.npz')
                if not (folder/'base-quality.npz').exists():
                    progress('基本メッシュを生成',region=name,error=error);bp,bf=base_mesh(r,binary,folder,error=error);np.savez(folder/'base-quality.npz',p=bp,f=bf);del bp,bf
                with np.load(folder/'base-quality.npz') as data:p=data['p'];f=data['f']
                initial.append(len(p))
                if reuse_all:
                    output=folder/'detail.json';shutil.copy2(checkpoint_folder(name,error,perceptual=True)/'detail.json',output);outputs.append(output);del p,f
                    progress('削減前の追加結果を再利用',region=name,error=error);continue
                seed=folder/'seed-mesh.json';seed.write_text(json.dumps(dict(vertices=p[:,:2].astype(int).tolist(),triangles=f.tolist())),encoding='utf-8');del p,f
                slope=np.hypot(r['gx'],r['gy']);gain=1.+.5*np.clip(slope/.5,0,1)+.5*np.clip(r['terrain_weight'],0,1)
                if perceptual:
                    from detail_priority import priority_gain
                    gain=priority_gain(r['grid'],r['terrain_weight'],r['valid'])
                if shape:
                    from detail_priority import shape_priority_gain
                    gain=shape_priority_gain(r['grid'],r['terrain_weight'],r['valid'])
                field_scale=max(.01,error/(60000*(.02 if perceptual else 1.)));limits=np.clip(np.rint(error/gain/field_scale),1,65535).astype(np.uint16);field=folder/'additional-limits.png';Image.fromarray(limits).save(field);del slope,gain,limits
                output=folder/'detail.json';outputs.append(output);log=open(folder/'detail.err','w',encoding='utf-8');logs.append(log)
                command=[str(binary),'--input',str(r['folder']/'height.png'),'--output',str(output),'--scale','.25','--offset',str(r['height_offset']),'--seed-mesh',str(seed),'--error-limits',str(field),'--detail-client','--detail-radius','128','--detail-density',str(4096 if perceptual else 256),'--detail-error',str(error),'--error-limit-scale',str(field_scale),'--max-vertices',str(r['grid'].size)]
                if minimum_additional_error:command+=['--detail-min-error',str(minimum_additional_error)]
                if shape:command+=['--detail-normal-spacing','31.25']
                progress('追加生成クライアントを初期化',region=name,base=initial[-1]);client=subprocess.Popen(command,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=log,text=True,encoding='utf-8');clients.append(client);row=client.stdout.readline()
                if not row:raise RuntimeError((folder/'detail.err').read_text()[-2000:])
                ready.append(json.loads(row))
            minimum_inserted_error=None
            base_total=sum(initial);budget=base_total//5;remaining=budget;heap=[];steps=[0]*len(regions);trace=[]
            if reuse_all:
                previous_mode='error-detail20-perceptual'+('' if error==25 else '-e'+format(error,'g'))
                with closing(sqlite3.connect(ROOT/f'build/terrain-shared-allocation-{previous_mode}.sqlite3')) as old:
                    previous=json.loads(old.execute('select value from metadata where key="manifest"').fetchone()[0])['sharedAllocation']
                assert previous['errorDetailPolicy']['basicVertices']==base_total
                steps=[previous['meshQuality']['regions'][r['area']['id']]['addedBeforePruning'] for r in regions];remaining=budget-sum(steps);assert remaining>=0
            for i,v in enumerate(ready):
                if v['priority']>1e-8:heapq.heappush(heap,(-v['priority'],i,v['sample']))
            progress('追加配分開始',basic=base_total,budget=budget)
            while remaining and heap:
                score,i,sample=heapq.heappop(heap)
                # 見た目への寄与が小さい候補だけになったら予算を使い切らない。
                if perceptual and -score<.1:break
                reply=request(clients[i],'step');assert reply['inserted']
                if minimum_additional_error:
                    assert reply['heightError']>minimum_additional_error
                    minimum_inserted_error=min(minimum_inserted_error or float('inf'),reply['heightError'])
                steps[i]+=1;remaining-=1
                if reply['priority']>1e-8:heapq.heappush(heap,(-reply['priority'],i,reply['sample']))
                if (budget-remaining)%max(1,min(1024,budget//10))==0:
                    row=dict(used=budget-remaining,budget=budget,byRegion=steps.copy(),priority=-score);trace.append(row);progress('追加頂点を配分',**row)
            # 全クライアントを保存・終了してからPythonの局所削減を行い、常駐メモリーを減らす。
            for client in clients:request(client,'save');client.stdin.close();client.wait(timeout=30)
            for i,(r,output) in enumerate(zip(regions,outputs)):
                name=r['area']['id'];folder=output.parent;checkpoint=folder/('mesh.npz' if no_prune else 'pruned.npz');data=json.loads(output.read_text(encoding='utf-8'));p=np.asarray(data['vertices'],float);f=np.asarray(data['triangles'],np.uint32);del data
                assert len(p)==initial[i]+steps[i];assert angles(p,f).min()>=10.-1e-6
                if no_prune:
                    ids=np.arange(len(p));pruning=dict(enabled=False,removed=0,trials=0,maximumSurfaceChange=0.,loopOneStepHeightChange=0.)
                    np.savez(checkpoint,p=p,f=f,ids=ids)
                elif checkpoint.exists():
                    with np.load(checkpoint) as saved:p=saved['p'];f=saved['f'];ids=saved['ids']
                    pruning=json.loads((folder/'pruning.json').read_text())
                else:
                    progress('不要頂点を局所削減',region=name,vertices=len(p));p,f,ids,pruning=prune(p,f,r['grid'],error=error,change_limit=error*.1,notify=lambda text:progress(text,region=name));np.savez(checkpoint,p=p,f=f,ids=ids);(folder/'pruning.json').write_text(json.dumps(pruning))
                delta=residual(p,f,r['grid']);v=delta[r['valid']];maximum=float(np.abs(delta).max());angle=angles(p,f);assert maximum<=error+1e-5;assert angle.min()>=10.-1e-6
                mesh=dict(p=p,f=f,maximum=maximum,rmse=float(np.sqrt(np.mean(v*v))),weightedMaximum=maximum/error,score=maximum/error);meshes.append(mesh)
                kept_base=int((ids<initial[i]).sum());reports[name]=dict(sourceVertices=len(p),initialVertices=kept_base,basicVerticesBeforePruning=initial[i],addedBeforePruning=steps[i],addedFromPool=int((ids>=initial[i]).sum()),pruning=pruning,maximum=maximum,rmse=mesh['rmse'],minimumAngle=float(angle.min()),under10=int((angle<10.-1e-6).sum()),beforeUnder10=0)
                progress('地域を検証',region=name,**reports[name])
            save('balanced',regions,meshes,manifest,base_total+budget,trace,variant=MODE,rebuild_topology=False)
        with closing(sqlite3.connect(path)) as db,db:
            m=json.loads(db.execute('select value from metadata where key="manifest"').fetchone()[0]);m['levels'][0]['maxErrorMeters']=error;topology=[];db.execute('CREATE TABLE IF NOT EXISTS topology(level,x,y,data,PRIMARY KEY(level,x,y))')
            for area,mesh in zip(m['reliefRegions'],meshes):
                keys=area['levels'][0]['keys'];ox=min(v[1] for v in keys);oy=min(v[2] for v in keys);records={tuple(v) for v in keys};world=mesh['p'].copy();world[:,:2]*=31.25;ranks=np.arange(len(world))/max(1,len(world)-1)
                for (x,y),body,count in tile_topology(world,mesh['f'],ranks=ranks):
                    key=(3,ox+x,oy+y)
                    if key in records:db.execute('insert or replace into topology values(?,?,?,?)',(*key,body))
                topology.append(dict(region=area['id'],source='direct-parent-tin',facesBefore=len(mesh['f']),facesAfter=len(mesh['f'])))
            a=m['sharedAllocation'];a['topology']='uncut-halo-v3';a['topologyReports']=topology;a['errorDetailPolicy']=dict(baseErrorMeters=error,minimumAngleDegrees=10,additionalFraction=.2,basicVertices=base_total,additionalBudget=budget,additionalUsed=budget-remaining,remaining=remaining,densityRadiusMeters=4000,densityReferenceVertices=4096 if perceptual else 256,densityEvaluationGridMeters=500,priority='(relativeHeightError+normalDifference/10deg)*terrainGain/(1+nearbyVertices/4096)' if shape else 'error*perceptualGain/(1+nearbyVertices/4096)' if perceptual else 'error*(1+0.5*slope+0.5*landform)/(1+nearbyVertices/256)',perceptualPriority=perceptual,shapePriority=shape,minimumAdditionalHeightErrorMeters=minimum_additional_error,minimumAdditionalHeightErrorRule='max(10,baseError/4)' if scaled_cutoff else 'fixed',minimumInsertedHeightErrorMeters=minimum_inserted_error,minimumAdditionalPriority=.1 if perceptual else 1e-8,pruningEnabled=not no_prune,pruneMaximumSurfaceChangeMeters=0. if no_prune else error*.1,mixedSizesShareParent=True)
            a['globalPool']=dict(totalBudget=base_total+budget,initialSurplus=budget,allocatedSurplus=budget-remaining,unusedPool=remaining);a['meshQuality']=dict(regions=reports);a['generationComplete']=True;update_metrics(db,m)
        verified=verify(MODE);from mixed_storage_mesh import generate;generate(MODE,error,checkpoint_prefix='terrain-error20-direct' if no_prune else 'terrain-error20-cutoff' if scaled_cutoff else 'terrain-error20-cut10' if minimum_additional_error else 'terrain-error20-shape' if shape else 'terrain-error20-perceptual' if perceptual else None,checkpoint_name='mesh.npz' if no_prune else 'pruned.npz')
        from verify_error_detail20 import verify_mixed
        mixed_verified=verify_mixed(MODE)
        progress('生成完了',complete=True,verified=verified,mixedVerified=mixed_verified,regions=reports,base=base_total,extraBudget=budget)
    finally:
        for client in clients:
            if client.poll() is None:
                client.stdin.close()
                try:client.wait(timeout=10)
                except subprocess.TimeoutExpired:client.kill();client.wait()
        for log in logs:log.close()

if __name__=='__main__':
    import argparse
    parser=argparse.ArgumentParser();parser.add_argument('--error',type=float,choices=[12.5,25.,50.,100.,200.,400.,800.],default=25.);parser.add_argument('--perceptual',action='store_true');parser.add_argument('--shape',action='store_true');parser.add_argument('--additional-min-error',type=float,choices=[0.,10.],default=0.);parser.add_argument('--scaled-cutoff',action='store_true');parser.add_argument('--no-prune',action='store_true');args=parser.parse_args();main(args.error,args.perceptual,args.shape,args.additional_min_error,args.scaled_cutoff,args.no_prune)
