"""保存済みz14標高PNGから、フィルターなしの元DEM比較データを返す。"""
import json,struct,threading
from collections import OrderedDict
from pathlib import Path
from functools import lru_cache
import numpy as np
from raw_dem_reference import STEP,center_for
from ridges import decode_png
from sibson_variable128 import LEVELS

lock=threading.RLock();cache=OrderedDict()
@lru_cache(maxsize=3)
def center(region):return np.asarray(center_for(region))
def raster(x,y):
 key=(int(x),int(y))
 with lock:
  if key in cache:cache.move_to_end(key);return cache[key]
  path=Path(f'build/terrain-cache/14/{key[0]}/{key[1]}.png')
  data=decode_png(path.read_bytes()).astype(np.float32) if path.exists() else np.full((256,256),np.nan,np.float32)
  cache[key]=data
  while len(cache)>512:cache.popitem(last=False)
  return data
def sample(region,q):
 uv=(np.asarray(q)+center(region)-64000)/STEP-.5;ij=np.floor(uv).astype(np.int64);fraction=uv-ij;total=np.zeros(len(q));weight=np.zeros(len(q))
 for dx,dy in [(0,0),(1,0),(0,1),(1,1)]:
  x=ij[:,0]+dx;y=ij[:,1]+dy;codes=((x//256)<<32)|((y//256)&0xffffffff);unique,inverse=np.unique(codes,return_inverse=True);order=np.argsort(inverse,kind='stable');offsets=np.r_[0,np.cumsum(np.bincount(inverse))];values=np.full(len(q),np.nan)
  for index,code in enumerate(unique):
   tx=int(code>>32);ty=int(code&0xffffffff);ids=order[offsets[index]:offsets[index+1]];values[ids]=raster(tx,ty)[y[ids]%256,x[ids]%256]
  w=(fraction[:,0] if dx else 1-fraction[:,0])*(fraction[:,1] if dy else 1-fraction[:,1]);valid=np.isfinite(values);total[valid]+=w[valid]*values[valid];weight[valid]+=w[valid]
 return np.divide(total,weight,out=np.full(len(q),np.nan),where=weight>1e-8)
def packet(region,error,x,y,n):
 if error not in dict(LEVELS) or n not in [8,16,32,64,128,256]:raise ValueError('元DEM比較の指定が不正です')
 side=dict(LEVELS)[error];count=round(128000/side)
 if not (0<=x<count and 0<=y<count):raise ValueError('元DEMタイルが範囲外です')
 bounds=np.array([x*side,y*side,(x+1)*side,(y+1)*side]);field=np.clip(bounds+[-250,-250,250,250],0,128000);xx,yy=np.meshgrid(np.linspace(field[0],field[2],n+1),np.linspace(field[1],field[3],n+1));q=np.c_[xx.ravel(),yy.ravel()];all_values=sample(region,np.vstack((q,q+[1,0],q-[1,0],q+[0,1],q-[0,1]))).reshape(5,-1);h=all_values[0];valid=np.isfinite(h);gradient=[]
 for first,second in [(1,2),(3,4)]:
  a=all_values[first];b=all_values[second];gradient.append(np.where(np.isfinite(a)&np.isfinite(b),(a-b)/2,0))
 values=np.c_[np.where(valid,h,0),*gradient,valid.astype(float)].astype('<f4');data=np.tile(values,(1,2)).tobytes();origin=(bounds[:2]+bounds[2:])/2
 info=dict(region=region,error=error,x=x,y=y,n=n,side=side,origin=origin.tolist(),coreBounds=bounds.tolist(),worldFieldBounds=field.tolist(),bounds=(field-np.tile(origin,2)).tolist(),format='raw-DEM-reference-v1',sourceStep=STEP,noLPF=True,missingSamples=int((~valid).sum()),heightmapBytes=len(data))
 header=json.dumps(info).encode();header+=b' '*((-len(header))%4);return struct.pack('<I',len(header))+header+data
