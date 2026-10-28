"""円形パッチを使わず、元のSibson曲面に近いC1勾配を全域で同時に求める。"""
import time,ctypes
from pathlib import Path
import numpy as np
from scipy.sparse import csr_matrix,vstack,eye,diags
from scipy.sparse.linalg import lsmr
from scipy.stats import qmc
from sibson_c1 import SibsonC1
from weighted_sibson import WeightedSibson

def gradient_operator(q,a,p):
 rows=np.repeat(np.arange(len(q)),np.diff(a.indptr));ids=a.indices;lam=a.data;delta=q[rows]-p[ids,:2];r2=np.sum(delta*delta,axis=1);r=np.sqrt(r2);inv=lam/np.maximum(r,1e-30);den=np.bincount(rows,weights=inv,minlength=len(q));alpha=np.bincount(rows,weights=lam*r,minlength=len(q))/den;beta=np.bincount(rows,weights=lam*r2,minlength=len(q));factor=np.divide(beta,alpha+beta,out=np.zeros(len(q)),where=alpha+beta>1e-25);weight=inv/den[rows]*factor[rows];z0=a@p[:,2];xi=np.bincount(rows,weights=inv*p[ids,2],minlength=len(q))/den;constant=z0*(1-factor)+factor*xi
 columns=np.column_stack((ids*2,ids*2+1)).ravel();values=(weight[:,None]*delta).ravel();matrix=csr_matrix((values,(np.repeat(rows,2),columns)),shape=(len(q),len(p)*2));matrix.eliminate_zeros();return matrix,constant,z0

class GlobalC1(SibsonC1):
 def __init__(self,points,strength=.01,queries=None,progress=None):
  if not np.isfinite(strength) or strength<0:raise ValueError('傾きの正則化は0以上の有限値にしてください')
  started=time.perf_counter();p=np.asarray(points,float);basis=WeightedSibson(p);n=len(p);tri=basis.model.tri
  if queries is None:
   lo=np.maximum(p[:,:2].min(axis=0)+1e-5,[-8000,-8000]);hi=np.minimum(p[:,:2].max(axis=0)-1e-5,[8000,8000]);power=max(14,int(np.ceil(np.log2(n*8))));uniform=qmc.Sobol(2,scramble=True,seed=9124).random_base2(power)*(hi-lo)+lo
   # 三角形はサンプル配置だけに使用し、曲面は自然近傍座標で評価する。
   face=p[tri,:2];near=np.concatenate([face[:,i]*.75+face[:,(i+1)%3]*.125+face[:,(i+2)%3]*.125 for i in range(3)]);inside=np.all((near>=lo)&(near<=hi),axis=1);queries=np.vstack((uniform,near[inside]))
  matrices=[];rhs=[]
  for first in range(0,len(queries),16384):
   q=np.asarray(queries[first:first+16384]);a=basis.basis(q);matrix,c,z=gradient_operator(q,a,p);matrices.append(matrix);rhs.append(z-c)
  design=vstack(matrices,format='csr');target=np.concatenate(rhs)
  edges=np.unique(np.sort(np.vstack((tri[:,[0,1]],tri[:,[1,2]],tri[:,[2,0]])),axis=1),axis=0);distance=np.linalg.norm(p[edges[:,0],:2]-p[edges[:,1],:2],axis=1);typical=np.median(distance);distance=np.minimum(distance,typical*3)
  rows=np.repeat(np.arange(len(edges)*2),2);columns=np.stack((edges*2,edges*2+1),axis=1).reshape(-1);values=np.stack((np.repeat(distance,2),-np.repeat(distance,2)),axis=1).ravel();regular=csr_matrix((values,(rows,columns)),shape=(len(edges)*2,n*2))
  # 内部の局所的な山頂・谷底では勾配をゼロにし、標高と位置を保つ。
  hull=set(np.unique(basis.model.triangulation.convex_hull));fixed=[]
  for i in range(n):
   if i in hull:continue
   ids=basis.model.links[basis.model.off[i]:basis.model.off[i+1]];dz=p[ids,2]-p[i,2]
   if np.max(abs(dz))>1e-8 and (np.all(dz<=0) or np.all(dz>=0)):fixed.append(i)
  free=np.ones(n*2,bool)
  for i in fixed:free[2*i:2*i+2]=False
  system=vstack((design,regular*np.sqrt(strength),eye(n*2)*1e-5),format='csr')[:,free];right=np.r_[target,np.zeros(regular.shape[0]+n*2)]
  if progress:progress(dict(stage='solving',samples=len(queries),unknowns=int(free.sum()),nonzeros=system.nnz))
  scale=1/np.maximum(np.sqrt(np.asarray(system.power(2).sum(axis=0)).ravel()),1e-10);solution=lsmr(system@diags(scale),right,atol=1e-6,btol=1e-6,maxiter=300);g=np.zeros(n*2);g[free]=solution[0]*scale;g=g.reshape(n,2);super().__init__(p,gradient=g,basis_model=basis);residual=design@g.ravel()-target
  self.fit_stats=dict(columnPreconditioned=True,strength=strength,samples=len(queries),unknowns=int(free.sum()),fixedExtrema=len(fixed),iterations=int(solution[2]),solverStop=int(solution[1]),fitRmse=float(np.sqrt(np.mean(residual*residual))),seconds=time.perf_counter()-started)
  from sibson_aux_surface import AuxiliarySibsonSurface
  self.fast_model=AuxiliarySibsonSurface(p);self.gradient=np.ascontiguousarray(self.gradient);self.fast_lib=ctypes.CDLL(str(Path('build/sibson_global_c1_eval.dll').resolve()));d=np.ctypeslib.ndpointer(dtype=np.float64,flags='C_CONTIGUOUS');i=np.ctypeslib.ndpointer(dtype=np.int32,flags='C_CONTIGUOUS');self.fast_lib.sibson_aux_c1.argtypes=[d,d,d,i,i,d,i,i,i,d,d,i,ctypes.c_int,d,i];self.fast_lib.sibson_aux_c1.restype=ctypes.c_int
 def __call__(self,q):
  q=np.ascontiguousarray(q,dtype=float);m=self.fast_model;starts=np.ascontiguousarray(m.triangulation.find_simplex(q),dtype=np.int32)
  if np.any(starts<0):raise ValueError('制御点の凸包外です')
  out=np.empty(len(q));stats=np.zeros(3,np.int32);code=self.fast_lib.sibson_aux_c1(m.xy,m.z,self.gradient,m.tri,m.adj,m.circles,m.off,m.links,m.cell_off,m.cells,q,starts,len(q),out,stats)
  if code:raise ValueError('全域C1のネイティブ評価に失敗しました')
  fallback=~np.isfinite(out)
  if fallback.any():out[fallback]=super().__call__(q[fallback])
  return out
