"""広域TINの頂点予算を一部確保し、保存境界を作る前に鋭角面を補修する。"""
import json, sqlite3, struct, tempfile, zlib
from pathlib import Path
import numpy as np
from compare_shared_allocation import ROOT, prepare, save, residual, compare, grow
from refine_regional_quality import triangulate, quality_candidates, angles
from weighted_mesh_allocation import movable_priority
from verify_shared_allocation import verify


def refine(points, ranks, grid, budget, reserve=.2, minimum=10., batch=256):
    """外周を保持し、低順位点の枠を形状改善に使う。元データは変更しない。"""
    n=len(grid);p=np.asarray(points,float);ranks=np.asarray(ranks)
    boundary=np.any((p[:,:2]==0)|(p[:,:2]==n-1),axis=1)
    interior=np.flatnonzero(~boundary)
    # 元の順位が同じ場合も座標順で決まり、毎回同じ候補になる。
    order=interior[np.lexsort((p[interior,1],p[interior,0],ranks[interior]))]
    keep_count=max(int(budget*(1-reserve)),int(boundary.sum()))
    keep=np.concatenate([np.flatnonzero(boundary),order[:max(0,keep_count-int(boundary.sum()))]])
    if len(keep)>budget:raise ValueError('外周だけで頂点予算を超えています')
    result=p[keep].tolist();existing={tuple(v[:2]) for v in result};added=0
    while len(result)<budget:
        faces=triangulate(result)
        candidates=quality_candidates(result,faces,n,minimum, min(batch,budget-len(result)))
        if not candidates:break
        for x,y in candidates:
            if (x,y) not in existing:
                result.append([x,y,float(grid[y,x])]);existing.add((x,y));added+=1
    out=np.asarray(result);f=triangulate(out)
    assert len(out)<=budget and np.isfinite(out).all()
    return out,f,dict(qualityInserted=added,retainedBeforeQuality=len(keep),sourceVertices=len(out),under10=int((angles(out,f)<10).sum()),under5=int((angles(out,f)<5).sum()),minimumAngle=float(angles(out,f).min()))


def read_parent(db,area):
    keys=area['levels'][0]['keys'];ox=min(k[1] for k in keys);oy=min(k[2] for k in keys);lookup={}
    for l,x,y in keys:
        raw=zlib.decompress(db.execute('SELECT geometry FROM tiles WHERE level=? AND x=? AND y=?',(l,x,y)).fetchone()[0])
        _,_,_,nv,ni=struct.unpack_from('<4sHHII',raw)
        p=np.frombuffer(raw,'<f4',nv*3,16).reshape(-1,3).astype(float);p[:,:2]+=[(x-ox)*8000,(y-oy)*8000]
        ranks=np.frombuffer(raw,'<f4',nv,16+nv*12+ni*2)
        for v,r in zip(p,ranks):
            xy=v[:2]/31.25
            if np.max(np.abs(xy-np.rint(xy)))>1e-6:continue
            key=tuple(np.rint(xy).astype(int));old=lookup.get(key)
            lookup[key]=(v[2],max(float(r),old[1] if old else -1))
    keys=sorted(lookup);return np.array([[*k,lookup[k][0]] for k in keys]),np.array([lookup[k][1] for k in keys])


def main():
    compare.initialize();compare.national.BINARY=str(ROOT/'build/terrain-rust-incremental-target/release/terrain-adaptive-tin.exe');regions=[];meshes=[];reports={}
    with sqlite3.connect(ROOT/'build/terrain-shared-allocation-landform128.sqlite3') as source, tempfile.TemporaryDirectory(dir=ROOT/'build',prefix='shared-quality-') as folder:
        manifest=json.loads(source.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
        with sqlite3.connect(ROOT/'build/terrain-regional-priority-levels-v5.sqlite3') as template_db:
            templates=json.loads(template_db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])['reliefRegions']
        for area in manifest['reliefRegions']:
            name=area['id'];print('品質補修の準備',name,flush=True)
            template=next(v for v in templates if v['id']==name)
            region=prepare(template,source,Path(folder)/name,128000.,landform_aware=True)
            p,r=read_parent(source,area);before=triangulate(p);budget=area['levels'][0]['priorityMetrics']['sourceVertices']
            report=dict(beforeVertices=len(p),budget=budget,beforeUnder10=int((angles(p,before)<10).sum()),beforeUnder5=int((angles(p,before)<5).sum()))
            out,f,after=refine(p,r,region['grid'],budget);report.update(after)
            region['seed']=out[:,:2].astype(int).tolist()
            repaired=grow(region,'balanced',budget,force=True);out=repaired['p'];f=repaired['f']
            final_angles=angles(out,f);report.update(sourceVertices=len(out),under10=int((final_angles<10).sum()),under5=int((final_angles<5).sum()),minimumAngle=float(final_angles.min()))
            error=residual(out,f,region['grid']);v=np.abs(error[region['valid']]);report.update(rmse=float(np.sqrt(np.mean(v*v))),maximum=float(v.max()))
            score=movable_priority(error,region['shape_limits'],region['valid'])
            meshes.append(dict(p=out,f=f,score=score,maximum=report['maximum'],rmse=report['rmse'],weightedMaximum=float((np.abs(error)/region['limits'])[region['valid']].max())))
            regions.append(region);reports[name]=report;print(name,report,flush=True)
        save('balanced',regions,meshes,manifest,192000,[],variant='landform-quality128')
    verification=verify('landform-quality128')
    assert all(v['under10']<v['beforeUnder10'] for v in reports.values()), '鋭角面が改善していません'
    report=dict(complete=True,reserveFraction=.2,angleTarget=10,regions=reports,verification=verification)
    with sqlite3.connect(ROOT/'build/terrain-shared-allocation-landform-quality128.sqlite3') as db:
        m=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0]);m['sharedAllocation']['meshQuality']=dict(reserveFraction=.2,angleTarget=10,regions=reports)
        db.execute('UPDATE metadata SET value=? WHERE key="manifest"',(json.dumps(m),))
    (ROOT/'build/terrain-shared-quality-shape-report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
    print('品質補修と境界検証が完了',reports,flush=True)


if __name__=='__main__':main()
