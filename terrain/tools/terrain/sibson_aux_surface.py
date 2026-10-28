"""Sibsonを保ったまま、制御点のVoronoiセルを事前計算して再利用する。"""
import ctypes,hashlib,struct,zlib,time
from pathlib import Path
import numpy as np
from sibson_surface import SibsonSurface
HEADER=struct.Struct('<4sII32s32s')
class AuxiliarySibsonSurface(SibsonSurface):
 def __init__(self,points,blob=None):
  started=time.perf_counter();super().__init__(points)
  if blob is None:
   incident=[[] for _ in self.xy]
   for t,face in enumerate(self.tri):
    for v in face:incident[v].append(t)
   hull=set(np.unique(self.triangulation.convex_hull));offset=[0];ring=[]
   for v,ids in enumerate(incident):
    if v not in hull:
     delta=self.circles[ids,:2]-self.xy[v];order=np.argsort(np.arctan2(delta[:,1],delta[:,0]),kind='stable')
     # Qhullの数値誤差などで真の凸セルでない場合は、従来の半平面切断へ戻す。
     near=self.links[self.off[v]:self.off[v+1]];planes=self.xy[near]-self.xy[v];squared=np.sum(planes*planes,axis=1);violation=delta@planes.T-squared*.5
     if len(delta)>=3 and np.max(violation)<=1e-9*max(1.,float(squared.max())):ring.extend(np.asarray(ids)[order])
    offset.append(len(ring))
   self.cell_off=np.ascontiguousarray(offset,dtype=np.int32);self.ring=np.ascontiguousarray(ring,dtype=np.int32)
  else:
   raw=zlib.decompress(blob);magic,n,m,point_hash,tri_hash=HEADER.unpack_from(raw)
   if magic!=b'SAU1' or n!=len(self.xy) or point_hash!=self.point_hash() or tri_hash!=self.tri_hash():raise ValueError('補助情報と制御点・三角形が一致しません')
   if len(raw)!=HEADER.size+4*(n+1+m):raise ValueError('補助情報の長さが不正です')
   self.cell_off=np.frombuffer(raw,'<i4',n+1,HEADER.size).copy();self.ring=np.frombuffer(raw,'<i4',m,HEADER.size+4*(n+1)).copy()
   if self.cell_off[0]!=0 or self.cell_off[-1]!=m or (np.diff(self.cell_off)<0).any() or ((self.ring<0)|(self.ring>=len(self.tri))).any():raise ValueError('補助情報の参照が不正です')
  owners=np.repeat(np.arange(len(self.xy)),np.diff(self.cell_off));self.cells=np.ascontiguousarray(self.circles[self.ring,:2]-self.xy[owners]);self.aux_build_seconds=time.perf_counter()-started
  self.auxlib=ctypes.CDLL(str(Path('build/sibson_aux_eval.dll').resolve()));d=np.ctypeslib.ndpointer(dtype=np.float64,flags='C_CONTIGUOUS');i=np.ctypeslib.ndpointer(dtype=np.int32,flags='C_CONTIGUOUS')
  self.auxlib.sibson_aux.argtypes=[d,d,i,i,d,i,i,i,d,d,i,ctypes.c_int,d,i];self.auxlib.sibson_aux.restype=ctypes.c_int
  self.last_chunk_stats={};self.aux_bytes=self.cells.nbytes+self.cell_off.nbytes+self.ring.nbytes
 def point_hash(self):return hashlib.sha256(np.column_stack((self.xy,self.z)).astype('<f8').tobytes()).digest()
 def tri_hash(self):return hashlib.sha256(self.tri.astype('<i4').tobytes()).digest()
 def auxiliary_blob(self):return zlib.compress(HEADER.pack(b'SAU1',len(self.xy),len(self.ring),self.point_hash(),self.tri_hash())+self.cell_off.astype('<i4').tobytes()+self.ring.astype('<i4').tobytes(),6)
 def _evaluate_native(self,q,starts,values):
  stats=np.zeros(3,dtype=np.int32);code=self.auxlib.sibson_aux(self.xy,self.z,self.tri,self.adj,self.circles,self.off,self.links,self.cell_off,self.cells,q,starts,len(q),values,stats);self.last_chunk_stats=dict(fast=int(stats[0]),fallback=int(stats[1]),exact=int(stats[2]));return code
