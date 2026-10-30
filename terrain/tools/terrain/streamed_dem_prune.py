"""PNG区画のストリーム監査でSibson点削除のメモリーを制限する。"""
import heapq,time
from collections import OrderedDict
from scipy.spatial import QhullError
from sibson_surface import SibsonSurface
import numpy as np
from continuous_dem import tile,STEP
from sibson_aux_surface import AuxiliarySibsonSurface

class DemReference:
 def __init__(self,world,half,keys=None,read=tile,step=STEP,sample_cache_bytes=128*1024*1024,residual_cache_bytes=32*1024*1024):
  self.world=np.asarray(world);self.half=half;self.step=step;self.read=read;self.cache={}
  self.samples=OrderedDict();self.sample_bytes=0;self.sample_limit=sample_cache_bytes
  self.residuals=OrderedDict();self.residual_bytes=0;self.residual_limit=residual_cache_bytes;self.pending=OrderedDict()
  self.lo=np.ceil((self.world-half)/step-.5).astype(int);self.hi=np.floor((self.world+half)/step-.5).astype(int)
  self.keys=keys if keys is not None else [(x,y) for y in range(int(self.lo[1]//256),int(self.hi[1]//256)+1) for x in range(int(self.lo[0]//256),int(self.hi[0]//256)+1)]
 def block(self,key):
  if key in self.samples:
   self.samples.move_to_end(key);return self.samples[key]
  tx,ty=key;x0=max(int(self.lo[0]),tx*256);x1=min(int(self.hi[0])+1,(tx+1)*256);y0=max(int(self.lo[1]),ty*256);y1=min(int(self.hi[1])+1,(ty+1)*256)
  a=self.read(tx,ty)[y0-ty*256:y1-ty*256,x0-tx*256:x1-tx*256];yy,xx=np.nonzero(np.isfinite(a));q=np.column_stack(((xx+x0+.5)*self.step-self.world[0],(yy+y0+.5)*self.step-self.world[1]));z=a[yy,xx]
  size=q.nbytes+z.nbytes
  if size<=self.sample_limit:
   while self.samples and self.sample_bytes+size>self.sample_limit:
    _,(oldq,oldz)=self.samples.popitem(last=False);self.sample_bytes-=oldq.nbytes+oldz.nbytes
   self.samples[key]=(q,z);self.sample_bytes+=size
  return q,z
 def audit(self,model,box=None,limit=None,progress=lambda **v:None):
  updates={};self.pending=OrderedDict();pending_bytes=0
  if box is None:keys=self.keys
  else:
   low=np.floor((np.asarray(box[:2])+self.world)/(256*self.step)).astype(int);high=np.floor((np.asarray(box[2:])+self.world)/(256*self.step)).astype(int)
   keys=[k for k in self.keys if low[0]<=k[0]<=high[0] and low[1]<=k[1]<=high[1] and self.cache.get(k,{}).get('count',1)>0]
  for i,key in enumerate(keys):
   q,z=self.block(key)
   if len(z):
    previous=self.residuals.get(key) if box is not None else None
    if previous is not None:
     inside=np.all((q>=box[:2])&(q<=box[2:]),axis=1);e=previous.copy()
     if inside.any():e[inside]=abs(model(q[inside])-z[inside])
    else:e=abs(model(q)-z)
    if not np.isfinite(e).all():raise ValueError('非有限の補間値')
    row=dict(max=float(e.max()),sumsq=float(e@e),count=len(z))
    if e.nbytes<=self.residual_limit:
     while self.pending and pending_bytes+e.nbytes>self.residual_limit:
      _,old=self.pending.popitem(last=False);pending_bytes-=old.nbytes
     self.pending[key]=e;pending_bytes+=e.nbytes
   else:row=dict(max=0.,sumsq=0.,count=0)
   if limit is not None and row['max']>limit:self.pending.clear();return None
   updates[key]=row
   if i%32==0 or i+1==len(keys):progress(blocksDone=i+1,blocksTotal=len(keys))
  return updates
 def commit(self,updates):
  # 不採用の試行で、採用済み曲面の誤差配列を上書きしない。
  self.cache.update(updates)
  for key in updates:
   old=self.residuals.pop(key,None)
   if old is not None:self.residual_bytes-=old.nbytes
   value=self.pending.pop(key,None)
   if value is not None:
    while self.residuals and self.residual_bytes+value.nbytes>self.residual_limit:
     _,old=self.residuals.popitem(last=False);self.residual_bytes-=old.nbytes
    self.residuals[key]=value;self.residual_bytes+=value.nbytes
  self.pending.clear()
 def summary(self):
  count=sum(v['count'] for v in self.cache.values());return dict(max=max((v['max'] for v in self.cache.values()),default=0),rmse=(sum(v['sumsq'] for v in self.cache.values())/count)**.5 if count else 0,validPixels=count)

def removal_box(model,index,margin):
 # 制御点の自然近傍への寄与は、その点に接するDelaunay円の合併に含まれる。
 c=model.circles[np.any(model.tri==index,axis=1)];r=np.sqrt(np.maximum(0,c[:,2]));return [np.min(c[:,0]-r)-margin,np.min(c[:,1]-r)-margin,np.max(c[:,0]+r)+margin,np.max(c[:,1]+r)+margin]


def removal_score(model,index):
 # 削除点に挿入し直す際の自然近傍は、削除前のDelaunay隣接点に限られる。
 # 順位の評価だけをこの局所点集合で計算し、採否は従来どおり全体の曲面で監査する。
 neighbors=np.sort(model.links[model.off[index]:model.off[index+1]])
 query=model.xy[index:index+1]
 if len(neighbors)>=3:
  try:
   local=SibsonSurface(np.column_stack((model.xy[neighbors],model.z[neighbors])))
   face=int(local.triangulation.find_simplex(query)[0])
   if face>=0:
    transform=local.triangulation.transform[face];bary=transform[:2]@(query[0]-transform[2])
    # 凸包上・ほぼ退化した配置は、元の全体計算へ戻す。
    if min(*bary,1-bary.sum())>1e-10:
     value=float(local(query)[0])
     if np.isfinite(value):return abs(value-model.z[index])
  except (QhullError,ValueError,np.linalg.LinAlgError):pass
 mask=np.arange(len(model.xy))!=index
 trial=AuxiliarySibsonSurface(np.column_stack((model.xy[mask],model.z[mask])))
 return abs(float(trial(query)[0])-model.z[index])

def reduce_streamed(points,reference,error,protected,progress=lambda **v:None,active=None,local_ranking=True):
 active=np.ones(len(points),bool) if active is None else active.copy();versions=np.zeros(len(points),int);heap=[];started=time.monotonic();attempts=0
 def build(mask):return AuxiliarySibsonSurface(points[mask])
 model=build(active);progress(stage='initial-full-audit',points=int(active.sum())-4)
 reference.commit(reference.audit(model,progress=lambda **v:progress(stage='initial-full-audit',**v)));initial=reference.summary()
 if initial['max']>error:raise ValueError(f'削減元が誤差超過: {initial}')
 def enqueue(ids):
  ids=sorted(set(map(int,ids)));current=np.flatnonzero(active);indices=np.full(len(points),-1,int);indices[current]=np.arange(len(current))
  for j,k in enumerate(ids):
   versions[k]+=1
   if not active[k] or protected[k]:continue
   if local_ranking:score=removal_score(model,int(indices[k]))
   else:
    mask=active.copy();mask[k]=False;trial=build(mask);score=abs(float(trial(points[k:k+1,:2])[0])-points[k,2])
   heapq.heappush(heap,(score,k,versions[k]))
   if j%64==0:progress(stage='rank',rankDone=j,rankTotal=len(ids),points=int(active.sum())-4)
 enqueue(np.flatnonzero(active&~protected));final_pass=False
 while True:
  if not heap:
   if final_pass:break
   final_pass=True;enqueue(np.flatnonzero(active&~protected));continue
  _,k,version=heapq.heappop(heap)
  if not active[k] or version!=versions[k]:continue
  ids=np.flatnonzero(active);index=int(np.flatnonzero(ids==k)[0]);box=removal_box(model,index,reference.step*2);mask=active.copy();mask[k]=False;trial=build(mask);attempts+=1
  updates=reference.audit(trial,box,error)
  if updates is not None:
   near={index}
   for _ in range(2):near.update(int(v) for j in list(near) for v in model.links[model.off[j]:model.off[j+1]])
   active=mask;model=trial;reference.commit(updates);final_pass=False;enqueue(ids[list(near)])
   progress(stage='prune',points=int(active.sum())-4,removed=int(len(points)-active.sum()),attempted=attempts,seconds=time.monotonic()-started,active=active,**reference.summary())
  elif attempts%32==0:progress(stage='prune',points=int(active.sum())-4,attempted=attempts,seconds=time.monotonic()-started)
 progress(stage='final-full-audit',points=int(active.sum())-4)
 reference.commit(reference.audit(model,progress=lambda **v:progress(stage='final-full-audit',**v)));audit=reference.summary()
 if audit['max']>error:raise ValueError(f'最終全画素監査で誤差超過: {audit}')
 audit.update(initialPoints=len(points)-4,finalPoints=int(active.sum())-4,removed=int(len(points)-active.sum()),attempted=attempts,noFeasibleSingleRemoval=True,allDemPixelsAudited=True)
 return active,model,audit
