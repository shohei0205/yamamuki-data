"""原DEM全画素の検査を保持し、点追加で影響する範囲だけ再評価する。"""
import numpy as np
from raw_dem_reference import STEP
from sibson_shared_codec import xy_keys

class ScanCache:
 def __init__(self,raw,block=64):
  self.raw=raw;self.block=block;self.x,self.y,self.lx,self.ly=raw.axes()
  self.nx=(len(self.x)+block-1)//block;self.ny=(len(self.y)+block-1)//block
  self.maximum=np.zeros(self.nx*self.ny);self.positions=np.zeros((len(self.maximum),2));self.squared=np.zeros(len(self.maximum));self.count=np.zeros(len(self.maximum),np.int64);self.keys=None
 def changed(self,model,points):
  if self.keys is None:return np.arange(len(self.maximum))
  keys=xy_keys(points);new=~np.isin(keys,self.keys,assume_unique=True)
  faces=np.any(new[model.tri],axis=1)
  if faces.sum()>10000:return np.arange(len(self.maximum))
  dirty=np.zeros((self.ny,self.nx),bool)
  for x,y,r2 in model.circles[faces]:
   radius=np.sqrt(max(0,r2))+2*STEP
   lo=np.floor((np.array([x-radius,y-radius])-[self.lx[0],self.ly[0]])/(self.block*STEP)).astype(int)
   hi=np.floor((np.array([x+radius,y+radius])-[self.lx[0],self.ly[0]])/(self.block*STEP)).astype(int)
   lo=np.maximum(lo,0);hi=np.minimum(hi,[self.nx-1,self.ny-1])
   if np.all(lo<=hi):dirty[lo[1]:hi[1]+1,lo[0]:hi[0]+1]=True
  return np.flatnonzero(dirty.ravel())
 def evaluate(self,model,points,error,progress=lambda **kw:None):
  dirty=self.changed(model,points);self.maximum[dirty]=0;self.squared[dirty]=0;self.count[dirty]=0;visited=0
  for first in range(0,len(dirty),48):
   queries=[];heights=[];blocks=[]
   for index in dirty[first:first+48]:
    bx=index%self.nx;by=index//self.nx;sx=slice(bx*self.block,(bx+1)*self.block);sy=slice(by*self.block,(by+1)*self.block)
    xx,yy=np.meshgrid(self.lx[sx],self.ly[sy]);h=self.raw.grid[np.ix_(self.y[sy]-self.raw.top,self.x[sx]-self.raw.left)].ravel();valid=np.isfinite(h)
    queries.append(np.column_stack((xx.ravel(),yy.ravel()))[valid]);heights.append(h[valid]);blocks.append(np.full(int(valid.sum()),index,np.int32))
   q=np.vstack(queries);h=np.concatenate(heights);ids=np.concatenate(blocks)
   if len(q):
    err=abs(model(q)-h);np.maximum.at(self.maximum,ids,err)
    selected=np.flatnonzero(err>=self.maximum[ids]);self.positions[ids[selected]]=q[selected]
    squares=np.bincount(ids,weights=err*err,minlength=len(self.maximum));counts=np.bincount(ids,minlength=len(self.maximum))
    selected_blocks=dirty[first:first+48];self.squared[selected_blocks]=squares[selected_blocks];self.count[selected_blocks]=counts[selected_blocks];visited+=len(q)
   if first%768==0:progress(samples=int(self.count.sum()),updatedSamples=visited,dirtyBlocks=len(dirty),processedBlocks=min(first+48,len(dirty)),totalBlocks=len(self.maximum),measuredMaxError=float(self.maximum.max()))
  self.keys=xy_keys(points)
  candidates=np.flatnonzero(self.maximum>error*.98);candidates=candidates[np.argsort(-self.maximum[candidates],kind='stable')]
  if len(candidates):
   triangle=model.triangulation.find_simplex(self.positions[candidates]);_,first=np.unique(triangle,return_index=True);candidates=candidates[np.sort(first)[:50000]]
  total=int(self.count.sum());maximum=float(self.maximum.max());rms=float(np.sqrt(self.squared.sum()/max(1,total)))
  return maximum,rms,self.positions[candidates],total
