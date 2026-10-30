"""他レベルの点数を参照せず、領域4隅に最も近い有効DEM点を選ぶ。"""
import numpy as np

def corner_support(blocks,side,world):
 corners=np.asarray([[-1,-1],[1,-1],[-1,1],[1,1]],float)*side/2
 best=np.full(4,np.inf);points=np.full((4,3),np.nan)
 for xy,z in blocks:
  xy=np.asarray(xy);z=np.asarray(z);valid=np.isfinite(z)&np.all(np.isfinite(xy),axis=1);xy=xy[valid];z=z[valid]
  if not len(z):continue
  for k,corner in enumerate(corners):
   d=np.sum((xy-corner)**2,axis=1);i=np.argmin(d)
   if d[i]<best[k]:best[k]=d[i];points[k]=[*xy[i],z[i]]
 if not np.isfinite(points).all():raise ValueError('有効DEM点がありません')
 points[:,:2]=np.rint((points[:,:2]+world)/.0625)*.0625-world
 points[:,2]=np.rint(points[:,2]/.125)*.125
 # 海岸の欠損により同じ点が複数の隅に最も近くなる場合も重複登録しない。
 return points[np.sort(np.unique(points[:,:2],axis=0,return_index=True)[1])]
