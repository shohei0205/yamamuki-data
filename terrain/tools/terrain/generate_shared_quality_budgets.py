"""品質達成で終了する頂点上限方式を標準とし、使い切り方式も比較用に残す。"""
import argparse,json,os,sqlite3,subprocess,tempfile
from pathlib import Path
from contextlib import closing
import numpy as np
from refine_shared_quality import ROOT,prepare,compare,read_parent,refine,triangulate,angles,grow,save
from compare_shared_allocation import boundary_seeds
from verify_shared_allocation import verify
from audit_shared_mountain_quality import local_errors

BUDGETS=[12000,24000,48000,96000,384000,768000]
PROGRESS=ROOT/'build/terrain-quality-budgets.progress.json'
REPORT=ROOT/'build/terrain-quality-budgets-report.json'


def allocate(total,weights,boundaries):
    """外周と最低限の内部枠を確保し、残りを元の地域配分に比例させる。"""
    floors=np.asarray(boundaries,dtype=int)+64
    if floors.sum()>total:raise ValueError('外周を保持できない頂点予算です')
    weights=np.asarray(weights,dtype=float);remaining=total;targets=np.zeros(len(weights),dtype=int);active=np.ones(len(weights),dtype=bool)
    while active.any():
        ideal=weights[active]/weights[active].sum()*remaining;ids=np.flatnonzero(active)
        fixed=ids[ideal<floors[ids]]
        if not len(fixed):
            values=np.floor(ideal).astype(int);targets[ids]=values
            for i in np.argsort(-(ideal-values),kind='stable')[:remaining-values.sum()]:targets[ids[i]]+=1
            break
        targets[fixed]=floors[fixed];remaining-=int(floors[fixed].sum());active[fixed]=False
    assert targets.sum()==total and (targets>=floors).all()
    return targets.tolist()


def progress(stage,**extra):
    PROGRESS.write_text(json.dumps(dict(stage=stage,**extra),ensure_ascii=False,indent=2),encoding='utf-8')
    print(stage,extra,flush=True)


def spatial_seeds(points):
    """初期点は各方向へ散らして挿入し、長い面の繰り返し評価を減らす。"""
    xy=np.asarray(points)[:,:2].astype(np.uint64)
    keys=((xy[:,0]*2654435761)^(xy[:,1]*2246822519))&0xffffffff
    return xy[np.argsort(keys,kind='stable')].astype(int).tolist()


def budget_parent(points,ranks,grid,total):
    """保存交点を除き、低予算では地域外周だけを粗くする。内部タイル辺は制約にしない。"""
    n=len(grid);p=np.asarray(points);ranks=np.asarray(ranks)
    boundary=np.any((p[:,:2]==0)|(p[:,:2]==n-1),axis=1)
    inside=(~boundary)&(ranks>=0)
    tolerance=12.5*max(1.,np.sqrt(192000./total))
    edge=boundary_seeds(grid,tolerance)
    outer=np.array([[x,y,float(grid[y,x])] for x,y in edge])
    return np.concatenate([outer,p[inside]]),np.concatenate([np.full(len(outer),-1.),ranks[inside]]),len(outer),tolerance


def cached_prepare(template,source):
    """重い標高準備を再開時に繰り返さない。キャッシュはbuild内だけに保存する。"""
    cache=ROOT/'build/terrain-quality-source-cache-v1';cache.mkdir(exist_ok=True)
    name=template['id'];metadata=cache/f'{name}.json';arrays=cache/f'{name}.npz'
    if metadata.exists() and arrays.exists():
        r=json.loads(metadata.read_text(encoding='utf-8'));r['folder']=Path(r['folder']);r['area']=template
        with np.load(arrays) as data:r.update({key:data[key] for key in data.files})
        return r
    folder=Path(tempfile.mkdtemp(dir=cache,prefix=name+'-'))/'data'
    r=prepare(template,source,folder,128000.,landform_aware=True)
    keys=['grid','limits','shape_limits','gx','gy','valid','terrain_weight']
    np.savez(arrays,**{key:r[key] for key in keys})
    info={key:r[key] for key in ['width','key','offset','n','spacing','height_offset','shape_aware','landform_aware']};info['folder']=str(folder)
    metadata.write_text(json.dumps(info),encoding='utf-8')
    return r


def publish(total,reports,completed,budgets):
    mode=f'landform-quality128-b{total}';pending_mode=mode+'-pending'
    pending_path=ROOT/f'build/terrain-shared-allocation-{pending_mode}.sqlite3';path=ROOT/f'build/terrain-shared-allocation-{mode}.sqlite3'
    result=verify(pending_mode if pending_path.exists() else mode);assert result['totalSource']==total
    if pending_path.exists():
        with closing(sqlite3.connect(pending_path)) as db,db:
            m=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0]);a=m['sharedAllocation'];a['mode']=mode;a['meshQuality']=dict(reserveFraction=.2,angleTarget=10,regions=reports,budgetScale=total/192000,allocationPolicy='base-region-ratios-with-scaled-outer-boundary-and-64-interior-floor',sourcePointPolicy='exclude-negative-rank-storage-intersections',seedInsertion='spatially-distributed-deterministic-hash')
            db.execute('UPDATE metadata SET value=? WHERE key="manifest"',(json.dumps(m),))
        pending_path.replace(path)
    progress('ブラウザー検証',budget=total)
    node=Path.home()/'.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node.exe';env=os.environ.copy();env['NODE_PATH']=str(node.parents[1]/'node_modules');env['ALLOCATION_MODES']=mode
    subprocess.run([str(node),'tools/terrain/test_shared_allocation_view.cjs'],cwd=ROOT,env=env,check=True)
    result.update(mode=mode,regions=reports,browserVerified=True,sqliteBytes=path.stat().st_size,localErrors=local_errors(mode))
    completed[str(total)]=result
    REPORT.write_text(json.dumps(dict(complete=all(str(b) in completed for b in budgets),budgets=budgets,results=completed),ensure_ascii=False,indent=2),encoding='utf-8')
    progress('予算の生成と検証完了',budget=total,sqliteBytes=result['sqliteBytes'],completed=list(completed))


def saved_reports(total):
    for suffix in ['-pending','']:
        path=ROOT/f'build/terrain-shared-allocation-landform-quality128-b{total}{suffix}.sqlite3'
        if not path.exists():continue
        with closing(sqlite3.connect(path)) as db:
            row=db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()
            if not row:continue
            a=json.loads(row[0]).get('sharedAllocation',{});q=a.get('meshQuality',{})
            if a.get('sourceVertices')==total and a.get('topology')=='uncut-halo-v3' and q.get('sourcePointPolicy')=='exclude-negative-rank-storage-intersections':return q['regions']
    return None


def main(budgets,fill_budget=False):
    if not fill_budget:
        from generate_capped_quality import main as generate_capped
        for budget in budgets:generate_capped(budget)
        return
    compare.initialize();compare.national.BINARY=str(ROOT/'build/terrain-rust-incremental-target/release/terrain-adaptive-tin.exe')
    completed={}
    if REPORT.exists():completed=json.loads(REPORT.read_text(encoding='utf-8')).get('results',{})
    pending=[b for b in budgets if not completed.get(str(b),{}).get('browserVerified')]
    if not pending:print('すべて検証済みです',flush=True);return
    with sqlite3.connect(ROOT/'build/terrain-shared-allocation-landform128.sqlite3') as source:
        manifest=json.loads(source.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
        with sqlite3.connect(ROOT/'build/terrain-regional-priority-levels-v5.sqlite3') as template_db:
            templates=json.loads(template_db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])['reliefRegions']
        regions=[];parents=[];weights=[];boundaries=[]
        for area in manifest['reliefRegions']:
            name=area['id'];progress('標高を準備',region=name,pending=pending)
            r=cached_prepare(next(t for t in templates if t['id']==name),source)
            p,rank=read_parent(source,area);regions.append(r);parents.append((p,rank));weights.append(area['levels'][0]['priorityMetrics']['sourceVertices'])
            boundaries.append(int(np.any((p[:,:2]==0)|(p[:,:2]==r['n']-1),axis=1).sum()))
        for total in pending:
            restored=saved_reports(total)
            if restored is not None:
                publish(total,restored,completed,budgets);continue
            mode=f'landform-quality128-b{total}';pending_mode=mode+'-pending';meshes=[];reports={}
            selected=[budget_parent(p,rank,r['grid'],total) for r,(p,rank) in zip(regions,parents)]
            targets=allocate(total,weights,[v[2] for v in selected])
            for r,(parent,rank,boundary_count,boundary_tolerance),target in zip(regions,selected,targets):
                name=r['area']['id'];progress('角度を補修',budget=total,region=name,target=target,completed=list(completed))
                p=parent;order=rank
                if target>len(parent):
                    # 高密度版はまず指定数の誤差優先TINを作り、その20%を補修用に取り直す。
                    r['seed']=spatial_seeds(parent);base=grow(r,'balanced',target,force=True)
                    p=base['p'];order=np.arange(len(p),dtype=float)
                before=angles(p,triangulate(p))
                out,f,stats=refine(p,order,r['grid'],target)
                progress('標高誤差を修復',budget=total,region=name,seeds=len(out),target=target)
                r['seed']=spatial_seeds(out);mesh=grow(r,'balanced',target,force=True);a=angles(mesh['p'],mesh['f'])
                stats.update(boundaryToleranceMeters=boundary_tolerance,initialBoundaryVertices=boundary_count,budget=target,beforeVertices=len(p),beforeUnder10=int((before<10).sum()),beforeUnder5=int((before<5).sum()),sourceVertices=len(mesh['p']),under10=int((a<10).sum()),under5=int((a<5).sum()),minimumAngle=float(a.min()),rmse=mesh['rmse'],maximum=mesh['maximum'])
                meshes.append(mesh);reports[name]=stats;progress('地域完了',budget=total,region=name,metrics=stats)
            progress('保存と接続を生成',budget=total)
            save('balanced',regions,meshes,manifest,total,[],variant=pending_mode)
            publish(total,reports,completed,budgets)
    progress('全予算完了',budgets=budgets,completed=list(completed))


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--budgets',type=int,nargs='+',choices=BUDGETS,default=BUDGETS);parser.add_argument('--fill-budget',action='store_true',help='比較用の旧方式。予算を使い切る');args=parser.parse_args();main(sorted(set(args.budgets)),fill_budget=args.fill_budget)
