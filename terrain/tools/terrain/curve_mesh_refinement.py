"""曲面の辺をDEMと比較し、過剰な外挿を起こす位置へ追加シードを選ぶ。"""
import numpy as np
from scipy.ndimage import map_coordinates

def curve_candidates(points,triangles,grid,gx,gy,tolerance,maximum=128):
 points=np.asarray(points,dtype=float);triangles=np.asarray(triangles,dtype=int)
 edges=np.unique(np.sort(np.concatenate([triangles[:,[0,1]],triangles[:,[1,2]],triangles[:,[2,0]]]),axis=1),axis=0)
 a=points[edges[:,0]];b=points[edges[:,1]];delta=b[:,:2]-a[:,:2];n=len(grid)
 # 固定共有辺には独立した点を足さない。短い辺も評価するが同じ画素は追加しない。
 interior=~(((a[:,0]==b[:,0])&((a[:,0]==0)|(a[:,0]==n-1)))|((a[:,1]==b[:,1])&((a[:,1]==0)|(a[:,1]==n-1))))
 ga=np.column_stack([map_coordinates(gx,[a[:,1],a[:,0]],order=1),map_coordinates(gy,[a[:,1],a[:,0]],order=1)])
 gb=np.column_stack([map_coordinates(gx,[b[:,1],b[:,0]],order=1),map_coordinates(gy,[b[:,1],b[:,0]],order=1)])
 m0=(ga*delta).sum(axis=1);m1=(gb*delta).sum(axis=1);existing={(int(p[0]),int(p[1])) for p in points};best={}
 for t in [.25,.5,.75]:
  q=np.rint(a[:,:2]+delta*t).astype(int);d2=(delta*delta).sum(axis=1);u=np.clip(((q-a[:,:2])*delta).sum(axis=1)/np.maximum(d2,1e-20),0,1)
  curve=(2*u**3-3*u*u+1)*a[:,2]+(u**3-2*u*u+u)*m0+(-2*u**3+3*u*u)*b[:,2]+(u**3-u*u)*m1
  errors=np.abs(curve-grid[q[:,1],q[:,0]])
  for i in np.flatnonzero(interior&(errors>tolerance)):
   key=tuple(map(int,q[i]));
   if key in existing or key[0] in [0,n-1] or key[1] in [0,n-1]:continue
   best[key]=max(best.get(key,0),float(errors[i]))
 return [(xy,error) for xy,error in sorted(best.items(),key=lambda v:(-v[1],v[0]))[:maximum]]


def mesh_errors(points,triangles,grid,limits):
 """保存する面そのものの誤差を測る。再三角形分割は行わない。"""
 maximum=ratio=0.;n=len(grid)
 for face in triangles:
  a,b,c=np.asarray(points)[face];ab=b[:2]-a[:2];ac=c[:2]-a[:2];det=ab[0]*ac[1]-ab[1]*ac[0]
  lo=np.maximum(0,np.ceil(np.minimum(np.minimum(a[:2],b[:2]),c[:2]))).astype(int);hi=np.minimum(n-1,np.floor(np.maximum(np.maximum(a[:2],b[:2]),c[:2]))).astype(int)
  yy,xx=np.mgrid[lo[1]:hi[1]+1,lo[0]:hi[0]+1];u=((xx-a[0])*(c[1]-a[1])-(yy-a[1])*(c[0]-a[0]))/det;v=((b[0]-a[0])*(yy-a[1])-(b[1]-a[1])*(xx-a[0]))/det
  inside=(u>=-1e-8)&(v>=-1e-8)&(u+v<=1+1e-8)
  if not inside.any():continue
  err=np.abs(a[2]+u*(b[2]-a[2])+v*(c[2]-a[2])-grid[yy,xx])[inside]
  maximum=max(maximum,float(err.max()));ratio=max(ratio,float((err/limits[yy,xx][inside]).max()))
 return maximum,ratio
