"""既存TINの局所密度から、距離とともに徐々に緩む頂点被覆半径を作る。"""
import numpy as np
from scipy.ndimage import distance_transform_edt

def graded_sizes(points,faces,n,spacing,growth=.3):
 neighbors=[[] for _ in points]
 for a,b,c in faces:
  for i,j in [(a,b),(b,c),(c,a)]:
   length=float(np.linalg.norm(points[i,:2]-points[j,:2])*spacing);neighbors[i].append(length);neighbors[j].append(length)
 radius=np.array([np.median(v)*.6 if v else 512 for v in neighbors]);radius=np.clip(radius,64,512)
 bands=2.**np.ceil(np.log2(radius));sizes=np.full((n,n),512.)
 x=np.rint(points[:,0]).astype(int);y=np.rint(points[:,1]).astype(int)
 for band in np.unique(bands):
  mask=np.ones((n,n),dtype=bool);chosen=bands==band;mask[y[chosen],x[chosen]]=False
  distance=distance_transform_edt(mask)*spacing
  sizes=np.minimum(sizes,band+growth*distance)
 return np.maximum(1.,np.floor(sizes/spacing*100)/100)
