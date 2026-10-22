"""新仕様比較用の誤差制約メッシュと三角形品質の処理。"""
import json,subprocess
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree,Delaunay
from refine_regional_quality import angles,triangulate,quality_candidates
from compare_shared_allocation import residual


def repair(points,grid,minimum=10.,maximum_rounds=1500,notify=print):
    p=np.asarray(points,float);n=len(grid)
    for iteration in range(maximum_rounds):
        f=triangulate(p);a=angles(p,f);bad=np.flatnonzero(a<minimum-1e-7)
        if not len(bad):return p,f
        candidates=quality_candidates(p,f,n,minimum,2048)
        existing={tuple(v[:2]) for v in p};chosen=[]
        for xy in candidates:
            if xy not in existing:existing.add(xy);chosen.append(xy)
        if not chosen:
            for face in f[bad]:
                q=p[face,:2];edges=np.roll(q,-1,axis=0)-q;k=np.argmax((edges*edges).sum(axis=1));xy=tuple(np.rint((q[k]+q[(k+1)%3])/2).astype(int))
                if xy not in existing:existing.add(xy);chosen.append(xy)
        if not chosen:raise ValueError(f'角度補修が停止: {a.min()}度 / {len(bad)}面')
        xy=np.asarray(chosen,int);p=np.vstack([p,np.c_[xy,grid[xy[:,1],xy[:,0]]]])
        if iteration%10==0:notify(f'角度補修 {iteration}: {len(p)}点 / 鋭角{len(bad)}面 / 最小{a.min():.3f}度')
    raise ValueError('角度補修が収束しません')


def base_mesh(region,binary,folder,error=25.):
    folder.mkdir(exist_ok=True);out=folder/'base.json'
    if not out.exists():
        command=[str(binary),'--input',str(region['folder']/'height.png'),'--output',str(out),'--scale','.25','--offset',str(region['height_offset']),'--max-error',str(error),'--max-vertices',str(region['grid'].size)]
        subprocess.run(command,check=True)
    data=json.loads(out.read_text());assert data['toleranceReached'];p=np.asarray(data['vertices'],float)
    for iteration in range(100):
        p,f=repair(p,region['grid'],notify=lambda s:print(region['area']['id'],s,flush=True))
        delta=residual(p,f,region['grid']);maximum=float(np.abs(delta).max())
        print('基本形状',region['area']['id'],iteration,len(p),'最大差',maximum,flush=True)
        if maximum<=error+1e-6:return p,f
        yy,xx=np.nonzero(np.abs(delta)>error+1e-6);order=np.argsort(-np.abs(delta[yy,xx]),kind='stable');tri=Delaunay(p[:,:2]);owner=tri.find_simplex(np.c_[xx[order],yy[order]])
        _,first=np.unique(owner,return_index=True);chosen=order[first];xy=np.c_[xx[chosen],yy[chosen]];p=np.vstack([p,np.c_[xy,region['grid'][xy[:,1],xy[:,0]]]])
    raise ValueError('誤差と角度の同時条件が収束しません')

def prune(points,faces,grid,error=25.,change_limit=2.5,minimum=10.,notify=print):
    """局所再分割で誤差・角度・接続・Loop一段の高さ変化を確認して削除する。"""
    p=np.asarray(points,float);faces={i:tuple(map(int,t)) for i,t in enumerate(faces)};incident=[set() for _ in p]
    for i,t in faces.items():
        for v in t:incident[v].add(i)
    alive=np.ones(len(p),bool);next_id=len(faces);removed=0;trials=0;n=len(grid)
    def inside(xy,poly):
        out=np.zeros(len(xy),bool)
        for a,b in zip(poly,np.roll(poly,-1,axis=0)):
            out^=((a[1]>xy[:,1])!=(b[1]>xy[:,1]))&(xy[:,0]<(b[0]-a[0])*(xy[:,1]-a[1])/(b[1]-a[1]+1e-300)+a[0])
        return out
    def raster(tris,xx,yy):
        values=np.full(xx.shape,np.nan)
        for t in tris:
            a,b,c=p[t];ab=b[:2]-a[:2];ac=c[:2]-a[:2];det=ab[0]*ac[1]-ab[1]*ac[0]
            u=((xx-a[0])*ac[1]-(yy-a[1])*ac[0])/det;v=(ab[0]*(yy-a[1])-ab[1]*(xx-a[0]))/det;mask=(u>=-1e-9)&(v>=-1e-9)&(u+v<=1+1e-9)
            values[mask]=a[2]+u[mask]*(b[2]-a[2])+v[mask]*(c[2]-a[2])
        return values
    # 平面からのずれが小さい候補を先に試す。境界は削除しない。
    score=np.full(len(p),np.inf)
    for i,ids in enumerate(incident):
        if not ids or np.any((p[i,:2]==0)|(p[i,:2]==n-1)):continue
        ring={v for f in ids for v in faces[f] if v!=i}
        if len(ring)<3:continue
        q=p[list(ring)]-p[i];coef=np.linalg.lstsq(q[:,:2],q[:,2],rcond=None)[0];score[i]=np.max(np.abs(q[:,2]-q[:,:2]@coef))
    for v in np.argsort(score,kind='stable'):
        if score[v]>change_limit:break
        old_ids=set(incident[v]);old=np.asarray([faces[i] for i in old_ids],int);ring=sorted({i for t in old for i in t if i!=v})
        if len(ring)<3:continue
        q=p[ring,:2]-p[v,:2];ring=np.asarray(ring)[np.argsort(np.arctan2(q[:,1],q[:,0]))];poly=p[ring,:2]
        if p[v,2]>p[ring,2].max()+.25 or p[v,2]<p[ring,2].min()-.25:continue
        trials+=1
        try:new=ring[triangulate(p[ring])]
        except Exception:continue
        mids=p[new,:2].mean(axis=1);new=new[inside(mids,poly)]
        if not len(new) or angles(p,new).min()<minimum-1e-7:continue
        edge_counts={}
        for t in new:
            for a,b in zip(t,np.roll(t,-1)):
                k=tuple(sorted((int(a),int(b))));edge_counts[k]=edge_counts.get(k,0)+1
        boundary={tuple(sorted((int(a),int(b)))) for a,b in zip(ring,np.roll(ring,-1))}
        if {e for e,c in edge_counts.items() if c==1}!=boundary:continue
        # 外部の隣接面とのDelaunay条件も守る。
        delaunay_ok=True
        for t in new:
            a,b,c=p[t,:2];u=b-a;w=c-a;det=2*(u[0]*w[1]-u[1]*w[0]);center=a+np.array([(u@u*w[1]-w@w*u[1])/det,(u[0]*(w@w)-w[0]*(u@u))/det]);r2=((a-center)**2).sum()
            for x,y in zip(t,np.roll(t,-1)):
                if tuple(sorted((int(x),int(y)))) not in boundary:continue
                for fid in (incident[x]&incident[y])-old_ids:
                    z=next(i for i in faces[fid] if i not in (x,y))
                    if ((p[z,:2]-center)**2).sum()<r2-1e-7:delaunay_ok=False;break
                if not delaunay_ok:break
            if not delaunay_ok:break
        if not delaunay_ok:continue
        lo=np.floor(poly.min(axis=0)).astype(int);hi=np.ceil(poly.max(axis=0)).astype(int)
        yy,xx=np.mgrid[lo[1]:hi[1]+1,lo[0]:hi[0]+1];before=raster(old,xx,yy);after=raster(new,xx,yy);mask=np.isfinite(before)
        if not np.isfinite(after[mask]).all() or np.max(np.abs(after[mask]-grid[yy[mask],xx[mask]]))>error+1e-6 or np.max(np.abs(after[mask]-before[mask]))>change_limit:continue
        # Loopで移動する周辺頂点の高さも、削除前後で大きく変えない。
        loop_ok=True
        for i in ring:
            old_neighbors={j for fid in incident[i] for j in faces[fid] if j!=i};new_neighbors={j for fid in incident[i]-old_ids for j in faces[fid] if j!=i}|{int(j) for t in new if i in t for j in t if j!=i}
            def loop_z(ids):
                count=len(ids);beta=(5/8-(3/8+.25*np.cos(2*np.pi/count))**2)/count;return (1-count*beta)*p[i,2]+beta*p[list(ids),2].sum()
            if abs(loop_z(old_neighbors)-loop_z(new_neighbors))>change_limit:loop_ok=False;break
        if not loop_ok:continue
        for fid in old_ids:
            for i in faces.pop(fid):incident[i].remove(fid)
        for t in new:
            faces[next_id]=tuple(map(int,t))
            for i in t:incident[i].add(next_id)
            next_id+=1
        alive[v]=False;removed+=1
        if removed%1000==0:notify(f'不要頂点削減 {removed}点 / 試行{trials}')
    ids=np.flatnonzero(alive);remap=np.zeros(len(p),int);remap[ids]=np.arange(len(ids));result=np.asarray(list(faces.values()),np.uint32)
    return p[ids],remap[result],ids,dict(removed=removed,trials=trials,maximumSurfaceChange=change_limit,loopOneStepHeightChange=change_limit)
