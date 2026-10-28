"""Sibsonの面積座標と制御点XYZから推定した勾配でC1曲面を比較する。"""
import numpy as np
from scipy.spatial import cKDTree,QhullError
from weighted_sibson import WeightedSibson

def gradients(points,limited=False):
 p=np.asarray(points,float);base=WeightedSibson(p);tree=cKDTree(p[:,:2]);distance,ids=tree.query(p[:,:2],k=min(17,len(p)));result=np.zeros((len(p),2));factors=np.ones(len(p))
 for i in range(len(p)):
  near=ids[i,1:];delta=p[near,:2]-p[i,:2];dz=p[near,2]-p[i,2];scale=max(float(distance[i,-1]),1e-12);d=delta/scale;w=1/np.maximum(distance[i,1:]/scale,.05);design=np.column_stack((d,d[:,0]**2,d[:,0]*d[:,1],d[:,1]**2));coef,_,rank,_=np.linalg.lstsq(design*w[:,None],dz*w,rcond=1e-10)
  if rank<5:coef=np.linalg.lstsq(d*w[:,None],dz*w,rcond=1e-10)[0]
  g=coef[:2]/scale
  # Sibsonの勾配推定：中心点を除いた自然近傍座標を距離の二乗で割る。
  adjacent=base.model.links[base.model.off[i]:base.model.off[i+1]]
  if len(adjacent)>=3:
   try:
    local=WeightedSibson(p[adjacent]);coordinates=local.basis(p[i:i+1,:2]).toarray().ravel();dxy=p[adjacent,:2]-p[i,:2];dr=np.linalg.norm(dxy,axis=1);local_scale=max(float(dr.max()),1e-12);local_weight=np.sqrt(np.maximum(coordinates,0))/np.maximum(dr/local_scale,1e-12);g=np.linalg.lstsq(dxy/local_scale*local_weight[:,None],(p[adjacent,2]-p[i,2])*local_weight,rcond=1e-10)[0]/local_scale
   except (ValueError,QhullError):pass
  if limited:
   change=delta@g;low=min(0.,float(dz.min()));high=max(0.,float(dz.max()));ratios=np.ones(len(change));positive=change>1e-12;negative=change< -1e-12;ratios[positive]=high/change[positive];ratios[negative]=low/change[negative];factors[i]=max(0.,min(1.,float(ratios.min())));g*=factors[i]
  result[i]=g
 return result,factors

class SibsonC1:
 def __init__(self,points,gradient=None,limited=False,basis_model=None):
  self.points=np.asarray(points,float);self.model=WeightedSibson(self.points) if basis_model is None else basis_model
  if gradient is None:self.gradient,self.limiting=gradients(self.points,limited)
  else:
   self.gradient=np.asarray(gradient,float).copy();self.limiting=np.ones(len(points))
   if limited:
    _,near=cKDTree(self.points[:,:2]).query(self.points[:,:2],k=min(17,len(points)))
    for i in range(len(points)):
     ids=near[i,1:];delta=self.points[ids,:2]-self.points[i,:2];dz=self.points[ids,2]-self.points[i,2];change=delta@self.gradient[i];low=min(0.,float(dz.min()));high=max(0.,float(dz.max()));ratios=np.ones(len(change));positive=change>1e-12;negative=change< -1e-12;ratios[positive]=high/change[positive];ratios[negative]=low/change[negative];self.limiting[i]=max(0.,min(1.,float(ratios.min())));self.gradient[i]*=self.limiting[i]
  if self.gradient.shape!=(len(points),2) or not np.isfinite(self.gradient).all():raise ValueError('制御点の勾配が不正です')
 def from_basis(self,q,a):
  q=np.asarray(q,float);counts=np.diff(a.indptr);rows=np.repeat(np.arange(len(q)),counts);ids=a.indices;lam=a.data;delta=q[rows]-self.points[ids,:2];r2=np.sum(delta*delta,axis=1);r=np.sqrt(r2);safe=np.maximum(r,1e-30);inv=lam/safe;den=np.bincount(rows,weights=inv,minlength=len(q));alpha=np.bincount(rows,weights=lam*r,minlength=len(q))/den;beta=np.bincount(rows,weights=lam*r2,minlength=len(q));z0=a@self.points[:,2];tangent=self.points[ids,2]+np.sum(self.gradient[ids]*delta,axis=1);xi=np.bincount(rows,weights=inv*tangent,minlength=len(q))/den;total=alpha+beta;h=np.divide(alpha*z0+beta*xi,total,out=z0.copy(),where=total>1e-25);exact=r<1e-10
  if exact.any():h[rows[exact]]=self.points[ids[exact],2]
  return h
 def __call__(self,q):
  q=np.asarray(q,float);out=np.empty(len(q))
  for first in range(0,len(q),16384):
   b=q[first:first+16384];out[first:first+len(b)]=self.from_basis(b,self.model.basis(b))
  return out
