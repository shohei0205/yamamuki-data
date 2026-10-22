"""小予算では挿入ごとに形状を再評価し、角度を崩さない誤差点だけを追加する。"""
import numpy as np
from scipy.spatial import Delaunay
from refine_shared_quality import refine,angles,triangulate


def graded_mesh(points,ranks,grid,budget,limits=None,minimum=10.,reserve=.25):
    p,f,report=refine(points,ranks,grid,budget,reserve=reserve,minimum=minimum,batch=1)
    if angles(p,f).min()<minimum-1e-8:raise ValueError('角度補修の枠が足りません')
    n=len(grid);pool=np.asarray(points,float);pool=pool[np.all((pool[:,:2]>0)&(pool[:,:2]<n-1),axis=1)];added=0;trials=0
    while len(p)<budget:
        tri=Delaunay(p[:,:2]);ids=tri.find_simplex(pool[:,:2]);delta=pool[:,:2]-tri.transform[ids,2];w=np.einsum('nij,nj->ni',tri.transform[ids,:2],delta);w=np.c_[w,1-w.sum(axis=1)]
        error=np.abs((p[tri.simplices[ids],2]*w).sum(axis=1)-pool[:,2])
        if limits is not None:error/=limits[pool[:,1].astype(int),pool[:,0].astype(int)]
        # 高誤差の候補を先に試す。入れられなければ大きい面の中心で密度差を緩和する。
        q=p[f,:2];ab=q[:,1]-q[:,0];ac=q[:,2]-q[:,0];area=np.abs(ab[:,0]*ac[:,1]-ab[:,1]*ac[:,0]);centers=np.rint(q.mean(axis=1))
        candidates=np.concatenate([pool[np.argsort(-error,kind='stable')[:512],:2],centers[np.argsort(-area,kind='stable')[:512]]])
        existing={tuple(v[:2]) for v in p};accepted=False
        for xy in candidates:
            x,y=map(int,xy)
            if (x,y) in existing or min(x,y)<=0 or max(x,y)>=n-1:continue
            trials+=1;candidate=np.vstack([p,[x,y,float(grid[y,x])]]);faces=triangulate(candidate)
            if angles(candidate,faces).min()<minimum-1e-8:continue
            p=candidate;f=faces;added+=1;accepted=True;break
        if not accepted:raise ValueError('角度目標を保って指定予算まで追加できません')
    a=angles(p,f);report.update(sourceVertices=len(p),under10=int((a<10).sum()),under5=int((a<5).sum()),minimumAngle=float(a.min()),safeInserted=added,candidateTrials=trials,reserveFraction=reserve,qualityBatch=1,minimumAngleGuard=minimum)
    return p,f,report
