"""鞍部から目立つピークを選び、頂上付近に誤差予算を配る試作。"""
import numpy as np
from scipy.ndimage import gaussian_filter,distance_transform_edt,maximum_filter,minimum_filter

def summit_limits(heights,valid,spacing,minimum_prominence=20.,peak_error=25.,flank_error=25.):
 if heights.ndim!=2 or valid.shape!=heights.shape or not np.isfinite(spacing) or spacing<=0:raise ValueError('標高配列と画素間隔が不正です')
 if not np.isfinite(heights[valid]).all():raise ValueError('有効標高に欠損があります')
 if not(0<peak_error<=flank_error and minimum_prominence>0):raise ValueError('ピークの誤差と鞍部からの高さが不正です')
 if not valid.any():raise ValueError('有効な標高がありません')
 filled=heights if valid.all() else heights[tuple(distance_transform_edt(~valid,return_distances=False,return_indices=True))]
 stride=max(1,int(round(62.5/spacing)));coarse=gaussian_filter(filled,sigma=max(.5,40/spacing),mode='nearest')[::stride,::stride]
 h,w=coarse.shape;z=coarse.ravel();parent=np.full(z.size,-1,dtype=np.int32);records=[]
 def find(a):
  while parent[a]!=a:parent[a]=parent[parent[a]];a=int(parent[a])
  return a
 # 高い側から流域を併合し、低い側のピークと合流する鞍部の差を記録する。
 for raw in np.argsort(-z,kind='stable'):
  at=int(raw);parent[at]=at;y,x=divmod(at,w)
  for dy in [-1,0,1]:
   for dx in [-1,0,1]:
    if not(dx or dy):continue
    yy=y+dy;xx=x+dx
    if not(0<=yy<h and 0<=xx<w):continue
    other=yy*w+xx
    if parent[other]<0:continue
    a=find(at);b=find(other)
    if a==b:continue
    if z[a]<z[b] or (z[a]==z[b] and a>b):a,b=b,a
    prominence=float(z[b]-z[at])
    if prominence>=minimum_prominence:records.append((b,prominence))
    parent[b]=a
 root=find(int(np.argmax(z)));prominence=float(z[root]-z.min())
 if prominence>=minimum_prominence:records.append((root,prominence))
 # 山全体を最大25mで残す。頂上だけに密度を集中させず、平坦部は10mとする。
 window=2*int(round(1000/spacing))+1
 local_range=maximum_filter(filled,size=window,mode='nearest')-minimum_filter(filled,size=window,mode='nearest')
 t=np.clip((local_range-20)/80,0,1);allowed=10+(flank_error-10)*t*t*(3-2*t)
 peaks=[];used=set();radius_samples=max(1,stride*2)
 for index,prominence in sorted(records,key=lambda r:(-r[1],r[0])):
  y,x=divmod(index,w);y*=stride;x*=stride
  ya=max(0,y-radius_samples);yb=min(heights.shape[0],y+radius_samples+1);xa=max(0,x-radius_samples);xb=min(heights.shape[1],x+radius_samples+1)
  patch=np.where(valid[ya:yb,xa:xb],heights[ya:yb,xa:xb],-np.inf);py,px=np.unravel_index(np.argmax(patch),patch.shape);y=int(ya+py);x=int(xa+px)
  if (x,y) in used or not valid[y,x]:continue
  used.add((x,y));height=float(heights[y,x]);radius=float(np.clip(600+prominence*.4,600,2400));drop=max(40.,prominence*.4)
  ya=max(0,y-int(np.ceil(radius/spacing)));yb=min(heights.shape[0],y+int(np.ceil(radius/spacing))+1);xa=max(0,x-int(np.ceil(radius/spacing)));xb=min(heights.shape[1],x+int(np.ceil(radius/spacing))+1)
  yy,xx=np.mgrid[ya:yb,xa:xb];radial=np.hypot(xx-x,yy-y)*spacing/radius;vertical=np.maximum(0,height-heights[ya:yb,xa:xb])/drop;t=np.clip((np.maximum(radial,vertical)-.4)/.6,0,1);blend=t*t*(3-2*t)
  limit=peak_error+(flank_error-peak_error)*blend*blend;allowed[ya:yb,xa:xb]=np.minimum(allowed[ya:yb,xa:xb],limit)
  peaks.append(dict(x=x,y=y,height=height,prominence=prominence,radiusMeters=radius,crownDropMeters=drop))
 return np.rint(allowed*100)/100,peaks
