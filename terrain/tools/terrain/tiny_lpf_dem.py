"""画素単位の小幅ガウシアンLPF。隣接PNGから余白を読む。"""
from functools import lru_cache
import numpy as np
from scipy.ndimage import gaussian_filter
from continuous_dem import tile,STEP

def filter_values(a,sigma):
 valid=np.isfinite(a);num=gaussian_filter(np.where(valid,a,0.),sigma,mode='constant',cval=0.,truncate=4.);den=gaussian_filter(valid.astype(float),sigma,mode='constant',cval=0.,truncate=4.);out=np.divide(num,den,out=np.full(a.shape,np.nan),where=den>1e-12);out[~valid]=np.nan;return out

@lru_cache(maxsize=128)
def filtered_tile(x,y,sigma):
 if sigma not in (.5,1.):raise ValueError('sigma must be .5 or 1 pixel')
 radius=int(4*sigma+.5);startx=x*256-radius;starty=y*256-radius;size=256+radius*2;a=np.full((size,size),np.nan)
 for ty in range(starty//256,(starty+size-1)//256+1):
  for tx in range(startx//256,(startx+size-1)//256+1):
   x0=max(startx,tx*256);x1=min(startx+size,(tx+1)*256);y0=max(starty,ty*256);y1=min(starty+size,(ty+1)*256)
   a[y0-starty:y1-starty,x0-startx:x1-startx]=tile(tx,ty)[y0-ty*256:y1-ty*256,x0-tx*256:x1-tx*256]
 return filter_values(a,sigma)[radius:radius+256,radius:radius+256].copy()

def pixels(x,y,sigma):
 x=np.asarray(x,dtype=np.int64);y=np.asarray(y,dtype=np.int64);out=np.full(x.shape,np.nan);keys,groups=np.unique(np.column_stack((x.ravel()//256,y.ravel()//256)),axis=0,return_inverse=True)
 for i,(tx,ty) in enumerate(keys):
  ids=np.flatnonzero(groups==i);out.ravel()[ids]=filtered_tile(int(tx),int(ty),sigma)[y.ravel()[ids]%256,x.ravel()[ids]%256]
 return out

def sample(q,sigma):
 xy=np.asarray(q)/STEP-.5;ij=np.floor(xy).astype(np.int64);f=xy-ij;out=np.zeros(len(q));den=np.zeros(len(q))
 for dx,dy,w in [(0,0,(1-f[:,0])*(1-f[:,1])),(1,0,f[:,0]*(1-f[:,1])),(0,1,(1-f[:,0])*f[:,1]),(1,1,f[:,0]*f[:,1])]:
  v=pixels(ij[:,0]+dx,ij[:,1]+dy,sigma);valid=np.isfinite(v);out+=np.where(valid,v,0)*w;den+=valid*w
 return np.divide(out,den,out=np.full(len(q),np.nan),where=den>1e-12)

def self_test():
 from unittest.mock import patch
 yy,xx=np.mgrid[:40,:40];ramp=2.*xx+3.*yy+10
 for sigma in (.5,1.):
  a=filter_values(ramp,sigma);assert np.allclose(a[8:-8,8:-8],ramp[8:-8,8:-8],atol=1e-10)
  constant=np.ones((30,30))*123;constant[14:16,14:16]=np.nan;a=filter_values(constant,sigma);assert np.isnan(a[14:16,14:16]).all() and np.allclose(a[np.isfinite(a)],123)
  checker=(-1.)**(xx+yy);a=filter_values(checker,sigma);assert abs(a[8:-8,8:-8]).max()<.4
  def synthetic(tx,ty):
   y,x=np.mgrid[:256,:256];return 2.*(x+tx*256)+3.*(y+ty*256)+10
  filtered_tile.cache_clear()
  with patch(__name__+'.tile',synthetic):
   a=filtered_tile(1,1,sigma);expected=synthetic(1,1);assert np.allclose(a,expected,atol=1e-8)
  filtered_tile.cache_clear()
 print('tiny LPF: affine, NoData, checkerboard, PNG boundaries PASS')
if __name__=='__main__':self_test()
