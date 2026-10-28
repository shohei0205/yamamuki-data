"""自由配置点から局所RBFを作り、コンパクトなC2重みで曲面を合成する。"""
import numpy as np
from scipy.interpolate import RBFInterpolator
from scipy.spatial import cKDTree

class PartitionRbfSurface:
 def __init__(self,points):
  points=np.asarray(points,dtype=np.float64)
  self.xy=points[:,:2];self.tree=cKDTree(self.xy)
  # 最遠点を順に選び、格子に依存しない重複領域を作る。
  count=min(4096,max(1,int(np.ceil(len(points)/24))))
  chosen=[int(np.argmin(np.sum((self.xy-self.xy.mean(axis=0))**2,axis=1)))];distance=np.full(len(points),np.inf)
  for _ in range(count-1):
   distance=np.minimum(distance,np.sum((self.xy-self.xy[chosen[-1]])**2,axis=1));chosen.append(int(np.argmax(distance)))
  centers=self.xy[chosen];center_tree=cKDTree(centers)
  coverage=float(center_tree.query(self.xy)[0].max())
  self.patches=[]
  for center in centers:
   distances,indices=self.tree.query(center,k=min(48,len(points)))
   distances=np.atleast_1d(distances);indices=np.atleast_1d(indices)
   scale=max(float(distances[-1]),1.)
   # 正規化により座標単位への依存と悪条件化を抑える。完全一致は要求しない。
   model=RBFInterpolator((self.xy[indices]-center)/scale,points[indices,2],kernel='thin_plate_spline',degree=1,smoothing=.002)
   radius=max(coverage*1.5,scale*.85,1.)
   self.patches.append((center,scale,radius,model))
 def __call__(self,query):
  q=np.asarray(query,dtype=np.float64);result=np.zeros(len(q));weights=np.zeros(len(q));tree=cKDTree(q)
  for center,scale,radius,model in self.patches:
   ids=np.asarray(tree.query_ball_point(center,radius),dtype=np.int64)
   if not len(ids):continue
   d=np.linalg.norm(q[ids]-center,axis=1)/radius
   w=(1-d)**4*(4*d+1)
   result[ids]+=w*model((q[ids]-center)/scale);weights[ids]+=w
  valid=weights>1e-20
  result[valid]/=weights[valid]
  # 配布領域外の要求は呼び出し側で制限する。重みゼロを隠さない。
  result[~valid]=np.nan
  return result
