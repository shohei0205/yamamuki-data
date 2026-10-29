"""元標高サンプルを保存する、対称な形状保持3次補間。LPFなし。"""
from functools import lru_cache
import numpy as np
from PIL import Image
from test_fixed_lpf_regions import ROOT,STEP
@lru_cache(maxsize=128)
def tile(x,y):
 path=ROOT/f'build/terrain-cache/14/{x}/{y}.png'
 if not path.exists():return np.full((256,256),np.nan)
 rgb=np.asarray(Image.open(path)).astype(np.int32);v=(rgb[:,:,0]<<16)|(rgb[:,:,1]<<8)|rgb[:,:,2];a=np.where(v>=8388608,v-16777216,v)*.01;a[v==8388608]=np.nan
 if rgb.shape[2]==4:a[rgb[:,:,3]==0]=np.nan
 return a

def pixels(x,y):
 x=np.asarray(x,dtype=np.int64);y=np.asarray(y,dtype=np.int64);out=np.full(x.shape,np.nan);keys,groups=np.unique(np.column_stack((x.ravel()//256,y.ravel()//256)),axis=0,return_inverse=True)
 for i,(tx,ty) in enumerate(keys):
  ids=np.flatnonzero(groups==i);out.ravel()[ids]=tile(int(tx),int(ty))[y.ravel()[ids]%256,x.ravel()[ids]%256]
 return out

def cubic(v,t):
 a,b,c,d=np.moveaxis(v,-1,0);d0=b-a;d1=c-b;d2=d-c
 def slope(x,y):
  valid=(x*y>0);return np.divide(2*x*y,x+y,out=np.zeros_like(x),where=valid)
 m0=slope(d0,d1);m1=slope(d1,d2);t2=t*t;t3=t2*t
 return (2*t3-3*t2+1)*b+(t3-2*t2+t)*m0+(-2*t3+3*t2)*c+(t3-t2)*m1

def sample(q):
 q=np.asarray(q);xy=q/STEP-.5;ij=np.floor(xy).astype(np.int64);f=xy-ij;n=len(q);v=np.empty((n,4,4))
 for y in range(4):
  for x in range(4):v[:,y,x]=pixels(ij[:,0]+x-1,ij[:,1]+y-1)
 xy_value=cubic(np.stack([cubic(v[:,y,:],f[:,0]) for y in range(4)],axis=-1),f[:,1]);yx_value=cubic(np.stack([cubic(v[:,:,x],f[:,1]) for x in range(4)],axis=-1),f[:,0]);out=(xy_value+yx_value)*.5
 # 欠損近傍は既存の有効画素だけのバイリニアへ戻す。海を0mにしない。
 invalid=~np.isfinite(out)
 if invalid.any():
  a=f[:,0];b=f[:,1];weights=np.stack(((1-a)*(1-b),a*(1-b),(1-a)*b,a*b),axis=-1);values=v[:,1:3,1:3].reshape(n,4);weights=np.where(np.isfinite(values),weights,0);den=weights.sum(axis=1);fallback=np.divide(np.nansum(values*weights,axis=1),den,out=np.full(n,np.nan),where=den>1e-12);out[invalid]=fallback[invalid]
 return out

def self_test():
 t=np.linspace(0,1,101);v=np.tile([0.,1.,2.,3.],(101,1));assert np.allclose(cubic(v,t),1+t)
 v=np.tile([-20.,1.,1.1,40.],(101,1));a=cubic(v,t);assert a.min()>=1-1e-12 and a.max()<=1.1+1e-12
 assert np.allclose(cubic(v,np.zeros(101)),1) and np.allclose(cubic(v,np.ones(101)),1.1)
 print('continuous DEM: affine / endpoints / no overshoot PASS')
if __name__=='__main__':self_test()
