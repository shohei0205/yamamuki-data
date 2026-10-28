"""Sibson 面積座標に正の点重みを掛ける比較用の有理補間。"""
import ctypes
from pathlib import Path
import numpy as np
from scipy.sparse import csr_matrix,diags
from scipy.sparse.linalg import lsmr
from sibson_surface import SibsonSurface

class WeightedSibson:
 def __init__(self,points,weights=None):
  self.model=SibsonSurface(points);self.points=np.asarray(points,dtype=float).copy()
  self.weights=np.ones(len(points)) if weights is None else np.asarray(weights,dtype=float).copy()
  if self.weights.shape!=(len(points),) or not np.isfinite(self.weights).all() or np.any(self.weights<=0):raise ValueError('重みは点ごとの正の有限値が必要です')
  self.lib=ctypes.CDLL(str(Path('build/sibson_weight_basis.dll').resolve()))
  d=np.ctypeslib.ndpointer(dtype=np.float64,flags='C_CONTIGUOUS');i=np.ctypeslib.ndpointer(dtype=np.int32,flags='C_CONTIGUOUS')
  self.lib.sibson_basis.argtypes=[d,i,i,d,i,i,d,i,ctypes.c_int,i,d,i];self.lib.sibson_basis.restype=ctypes.c_int
 def basis(self,query):
  q=np.ascontiguousarray(query,dtype=float);m=self.model;starts=np.ascontiguousarray(m.triangulation.find_simplex(q),dtype=np.int32)
  if np.any(starts<0):raise ValueError('制御点の凸包外です')
  rows=[];cols=[];values=[]
  boundary=np.any((q<=m.lower+1e-8)|(q>=m.upper-1e-8),axis=1)
  if boundary.any():
   ids=np.flatnonzero(boundary);t=m.triangulation.transform[starts[ids]];b=np.einsum('nij,nj->ni',t[:,:2,:],q[ids]-t[:,2,:]);v=np.column_stack((b,1-b.sum(axis=1)))
   rows.append(np.repeat(ids,3));cols.append(m.tri[starts[ids]].ravel());values.append(v.ravel())
  selected=np.flatnonzero(~boundary)
  for first in range(0,len(selected),2048):
   ix=selected[first:first+2048];n=len(ix);ids=np.empty((n,256),np.int32);w=np.empty((n,256));counts=np.empty(n,np.int32)
   code=self.lib.sibson_basis(m.xy,m.tri,m.adj,m.circles,m.off,m.links,np.ascontiguousarray(q[ix]),np.ascontiguousarray(starts[ix]),n,ids,w,counts)
   if code:raise ValueError(f'面積座標の計算に失敗: {ix[code-1]}')
   valid=np.arange(256)[None,:]<counts[:,None];r,c=np.nonzero(valid);rows.append(ix[r]);cols.append(ids[r,c]);values.append(w[r,c])
  a=csr_matrix((np.concatenate(values),(np.concatenate(rows),np.concatenate(cols))),shape=(len(q),len(m.xy)));a.eliminate_zeros();return a
 def __call__(self,q):
  a=self.basis(q);return (a@(self.weights*self.points[:,2]))/(a@self.weights)

def fit(a,target,initial,error,use_weights=True,iterations=12,z_regular=.03):
 """XYを固定し、Zと正の重みを交互に適合する。改善した試行だけを採用。"""
 z0=np.asarray(initial,dtype=float);z=z0.copy();logw=np.zeros(len(z));history=[]
 def objective(z,l):
  w=np.exp(l);res=((a@(w*z))/(a@w)-target)/error
  return float(res@res+z_regular*np.sum(((z-z0)/error)**2)+.1*np.sum(l*l))
 for iteration in range(iterations if use_weights else 1):
  w=np.exp(logw);den=a@w;b=diags(1/den)@a@diags(w)
  # Zの変位を抑え、点数の少ない領域や外周での過適合を防ぐ。
  from scipy.sparse import vstack,eye
  z=z0+lsmr(vstack((b,eye(len(z))*np.sqrt(z_regular))),np.r_[target-b@z0,np.zeros(len(z))],atol=1e-6,btol=1e-6,maxiter=150)[0]
  prediction=b@z;current=objective(z,logw);accepted=False
  if use_weights:
   j=(b@diags(z)-diags(prediction)@b)/error
   step=lsmr(vstack((j,eye(len(z))*np.sqrt(.1))),np.r_[(target-prediction)/error,-np.sqrt(.1)*logw],atol=1e-5,btol=1e-5,maxiter=80)[0]
   for rate in [1.,.5,.25,.125]:
    candidate=np.clip(logw+rate*step,np.log(.25),np.log(4.))
    if objective(z,candidate)<current:logw=candidate;accepted=True;break
  history.append(dict(iteration=iteration,objective=objective(z,logw),rmse=float(np.sqrt(np.mean(((a@(np.exp(logw)*z))/(a@np.exp(logw))-target)**2))),accepted=accepted))
  if use_weights and not accepted:break
 return z,np.exp(logw),history


def encode_weighted(points,weights):
 """XYZの空間順に重みをそろえ、別BLOBへ量子化する。"""
 import zlib
 from scipy.spatial import cKDTree
 from mls_quant_codec import encode_quant,decode_quant
 points=np.asarray(points,dtype=float);weights=np.asarray(weights,dtype=float)
 if weights.shape!=(len(points),) or not np.isfinite(weights).all() or np.any((weights<.25)|(weights>4)):raise ValueError('重みの保存範囲は0.25～4です')
 blob=encode_quant(points,xy=.0625,zstep=.125);restored=decode_quant(blob);distance,order=cKDTree(points[:,:2]).query(restored[:,:2])
 if len(np.unique(order))!=len(points) or np.max(distance)>.05:raise ValueError('XYZ量子化で点の対応が失われました')
 code=np.rint((np.log2(weights[order])+2)*32).clip(0,128).astype('u1')
 return blob,zlib.compress(code.tobytes(),6)

def decode_weighted(blob,weight_blob):
 import zlib
 from mls_quant_codec import decode_quant
 points=decode_quant(blob);code=np.frombuffer(zlib.decompress(weight_blob),'u1')
 if len(code)!=len(points) or np.any(code>128):raise ValueError('重みBLOBが不正です')
 return points,2**(code.astype(float)/32-2)
