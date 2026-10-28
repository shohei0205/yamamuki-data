"""自然近傍の制御点直近だけを丸め、離れた元曲面を厳密に維持する。"""
import time
import numpy as np
from scipy.spatial import cKDTree
from sibson_aux_surface import AuxiliarySibsonSurface

class RoundedSibson:
 def __init__(self,points,fraction=.35,base=None):
  if not 0<fraction<.5:raise ValueError('丸め半径の比率は0より大きく0.5未満にしてください')
  started=time.perf_counter();self.points=np.asarray(points,float);self.base=AuxiliarySibsonSurface(self.points) if base is None else base;self.tree=cKDTree(self.points[:,:2]);distance,_=self.tree.query(self.points[:,:2],k=2);self.radius=distance[:,1]*fraction;self.radius[np.unique(self.base.triangulation.convex_hull)]=0;self.coefficients=np.zeros((len(points),5));self.fraction=fraction
  angle=np.arange(32)*2*np.pi/32;directions=np.column_stack((np.cos(angle),np.sin(angle)));unit=np.vstack((directions*.5,directions));design=np.column_stack((unit,unit[:,0]**2,unit[:,0]*unit[:,1],unit[:,1]**2));inverse=np.linalg.pinv(design)
  active=np.flatnonzero(self.radius>1e-8)
  for first in range(0,len(active),1024):
   ids=active[first:first+1024];ring=self.points[ids,None,:2]+unit[None,:,:]*self.radius[ids,None,None];valid=np.all(self.base.triangulation.find_simplex(ring.reshape(-1,2)).reshape(len(ids),len(unit))>=0,axis=1);self.radius[ids[~valid]]=0;ids=ids[valid];ring=ring[valid]
   if not len(ids):continue
   value=self.base(ring.reshape(-1,2)).reshape(len(ids),len(unit))-self.points[ids,2,None];self.coefficients[ids]=value@inverse.T
  self.build_seconds=time.perf_counter()-started
 def __call__(self,q):
  return self.from_base(q,self.base(q))
 def from_base(self,q,height):
  q=np.asarray(q,float);height=np.asarray(height,float).copy();distance,ids=self.tree.query(q);active=(self.radius[ids]>0)&(distance<self.radius[ids])
  if active.any():
   selected=ids[active];d=(q[active]-self.points[selected,:2])/self.radius[selected,None];design=np.column_stack((d,d[:,0]**2,d[:,0]*d[:,1],d[:,1]**2));polynomial=self.points[selected,2]+np.sum(design*self.coefficients[selected],axis=1);t=distance[active]/self.radius[selected];blend=1-10*t**3+15*t**4-6*t**5;height[active]+=blend*(polynomial-height[active])
  return height
