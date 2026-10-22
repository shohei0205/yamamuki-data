"""誤差と面の品質を満たしたら終了し、指定予算は上限としてのみ使う。"""
import numpy as np
from refine_shared_quality import angles,triangulate
from refine_regional_quality import quality_candidates
from compare_shared_allocation import grow,residual
from weighted_mesh_allocation import movable_priority


def build_to_quality(region,cap,minimum=10.,batch=128,grow_mesh=grow):
    if cap<4:raise ValueError('最大頂点数は4以上必要です')
    n=len(region['grid']);initial=list(region['seed'])
    if len(initial)>cap:raise ValueError('外周頂点が上限を超えています')
    region=dict(region);region['seed']=initial
    def spread(seed):
        xy=np.asarray(seed,dtype=np.uint64);h=((xy[:,0]*2654435761)^(xy[:,1]*2246822519))&0xffffffff
        return xy[np.argsort(h,kind='stable')].astype(int).tolist()
    region['seed']=spread(initial)
    mesh=grow_mesh(region,'balanced',max(len(initial),int(cap*.75),4),force=False)
    quality_added=0;calls=1
    while True:
        p=mesh['p'];f=mesh['f'];a=angles(p,f)
        error=residual(p,f,region['grid'])
        limits=region.get('shape_limits')
        if limits is None:limits=region['limits']
        priority=movable_priority(error,limits,region['valid'])
        error_ok=priority<=1.+1e-6;angle_ok=float(a.min())>=minimum-1e-6
        if error_ok and angle_ok:reason='quality-reached';break
        if len(p)>=cap:reason='vertex-cap';break
        region['seed']=p[:,:2].astype(int).tolist()
        if not angle_ok:
            candidates=quality_candidates(p,f,n,minimum,min(32,cap-len(p)))
            if not candidates:reason='quality-stalled';break
            extra=np.array([[x,y,float(region['grid'][y,x])] for x,y in candidates])
            points=np.vstack([p,extra]);faces=triangulate(points);quality_added+=len(extra)
            delta=residual(points,faces,region['grid']);v=delta[region['valid']]
            mesh=dict(p=points,f=faces,maximum=float(np.abs(v).max()),rmse=float(np.sqrt(np.mean(v*v))),weightedMaximum=float(np.max(np.abs(v)/region['limits'][region['valid']])))
            continue
        target=min(cap,len(p)+batch);region['seed']=spread(region['seed'])
        previous=mesh;mesh=grow_mesh(region,'balanced',target,force=False);calls+=1
        if len(mesh['p'])<=len(p):mesh=previous;reason='error-stalled';break
    mesh.update(score=priority)
    report=dict(budget=cap,sourceVertices=len(p),unusedVertices=cap-len(p),stopReason=reason,
        errorReached=bool(error_ok),angleReached=bool(angle_ok),qualityReached=bool(error_ok and angle_ok),
        maximumNormalizedError=priority,minimumAngle=float(a.min()),under10=int((a<10).sum()),under5=int((a<5).sum()),
        qualityInserted=quality_added,growCalls=calls,maximum=mesh['maximum'],rmse=mesh['rmse'],
        angleTarget=minimum,normalizedErrorTarget=1.)
    return mesh,report
