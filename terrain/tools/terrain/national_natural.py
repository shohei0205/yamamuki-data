"""全国仕様案の座標・整数差分復元。GPUのfloat32化前に原点を移す。"""
import math,hashlib
import numpy as np
from sibson_native_codec import HEADER,decode_independent,decode_delta_arrays
WORLD=40075016.68557849
XY=.0625
Z=.125
def tile_width(z):
 if not 0<=z<=30:raise ValueError('不正なズーム')
 return WORLD/(1<<z)
def xyz(lon,lat,z):
 if not math.isfinite(lon) or not math.isfinite(lat) or not -180<=lon<=180 or not -85.0511287798066<=lat<=85.0511287798066:raise ValueError('Webメルカトルの範囲外')
 n=1<<z;x=math.floor((lon+180)/360*n);y=math.floor((1-math.asinh(math.tan(math.radians(lat)))/math.pi)/2*n);return max(0,min(n-1,x)),max(0,min(n-1,y))
def origin_q(tile):
 z,x,y=tile;n=1<<z
 if not 0<=x<n or not 0<=y<n:raise ValueError('タイル番号が範囲外')
 width=tile_width(z);return np.array([math.floor(x*width/XY+.5),math.floor(y*width/XY+.5)],np.int64)
def global_q_to_gpu(q,tile):
 q=np.asarray(q,np.int64);z,x,y=tile;width=tile_width(z)
 out=q.astype(np.float64)*[XY,XY,Z];out[:,:2]-=[(x+.5)*width,(y+.5)*width];return out.astype(np.float32)
def point_id(qx,qy):
 limit=math.ceil(WORLD/XY)
 if not 0<=qx<=limit or not 0<=qy<=limit:raise ValueError('全球座標範囲外')
 return (int(qx)<<32)|int(qy)
def decode_global_q(blob,tile,parents=()):
 magic,n,pc,nf,xy,z=HEADER.unpack_from(blob)
 if n>262144 or len(blob)>16777216 or pc>4 or xy!=XY or z!=Z or pc!=len(parents):raise ValueError('全国プロファイルの刻み・親数が不一致')
 origin=origin_q(tile)
 if magic==b'NQC1':local=decode_independent(blob);out=local.copy();out[:,:2]+=origin
 elif magic==b'NDC1':
  refs,delta,fresh=decode_delta_arrays(blob);new=refs==0xffffffff;out=np.empty((n,3),np.int64);out[new]=fresh;out[new,:2]+=origin
  for slot,parent in enumerate(parents):
   ids=np.flatnonzero((~new)&((refs>>24)==slot));indices=refs[ids]&0xffffff
   if len(indices) and indices.max()>=len(parent):raise ValueError('親参照が範囲外')
   out[ids]=np.asarray(parent,np.int64)[indices];out[ids,2]+=delta[ids]
 else:raise ValueError('未知の全国制御点形式')
 if np.any((out<np.iinfo(np.int32).min)|(out>np.iinfo(np.int32).max)):raise ValueError('座標がint32範囲外')
 return out.astype('<i4')
def xyz_digest(q):return hashlib.sha256(np.asarray(q,dtype='<i4').tobytes()).digest()
