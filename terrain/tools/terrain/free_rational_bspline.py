"""自由配置の局所3次Bスプライン基底による有理曲面。標準NURBS網ではない。"""
import struct,zlib
import numpy as np
from scipy.sparse import csr_matrix
from scipy.spatial import cKDTree
from mls_quant_codec import encode_quant,decode_quant

def beta(x):
 t=np.abs(x);return np.where(t<1,2/3-t*t+.5*t**3,np.where(t<2,(2-t)**3/6,0.))

class FreeRationalSpline:
 def __init__(self,points,scales,angles=None,weights=None):
  self.points=np.asarray(points,float);self.scales=np.asarray(scales,float);n=len(points)
  self.angles=np.zeros(n) if angles is None else np.asarray(angles,float);self.weights=np.ones(n) if weights is None else np.asarray(weights,float)
  if self.points.shape!=(n,3) or self.scales.shape!=(n,2) or not np.isfinite(self.points).all() or not np.isfinite(self.scales).all() or np.any(self.scales<=0) or self.angles.shape!=(n,) or not np.isfinite(self.angles).all() or self.weights.shape!=(n,) or not np.isfinite(self.weights).all() or np.any(self.weights<=0):raise ValueError('自由Bスプラインの制御点が不正です')
  self.tree=cKDTree(self.points[:,:2]);self.radius=2*np.linalg.norm(self.scales,axis=1)
  # 大小の影響範囲を別に探索し、最も広い点の半径で全点を探索しない。
  groups=np.floor(np.log2(self.radius)).astype(int);self.groups=[]
  for k in np.unique(groups):
   ids=np.flatnonzero(groups==k);self.groups.append((ids,cKDTree(self.points[ids,:2]),float(self.radius[ids].max())))
 def basis(self,q):
  q=np.asarray(q,float).reshape(-1,2);rows=[];cols=[];values=[]
  for first in range(0,len(q),4096):
   query=q[first:first+4096]
   for ids,tree,radius in self.groups:
    lists=tree.query_ball_point(query,radius);counts=np.array([len(v) for v in lists]);r=np.repeat(np.arange(len(query)),counts)
    if not len(r):continue
    c=ids[np.concatenate(lists).astype(int)];delta=query[r]-self.points[c,:2];co=np.cos(self.angles[c]);si=np.sin(self.angles[c]);u=(co*delta[:,0]+si*delta[:,1])/self.scales[c,0];v=(-si*delta[:,0]+co*delta[:,1])/self.scales[c,1];b=beta(u)*beta(v);keep=b>1e-18
    rows.append(r[keep]+first);cols.append(c[keep]);values.append(b[keep])
  a=csr_matrix((np.concatenate(values),(np.concatenate(rows),np.concatenate(cols))),shape=(len(q),len(self.points)))
  total=a.sum(axis=1).A.ravel()
  if np.any(total<1e-14):raise ValueError('Bスプラインの支持域が照会範囲を覆っていません')
  from scipy.sparse import diags
  return diags(1/total)@a
 def __call__(self,q):
  a=self.basis(q);return (a@(self.weights*self.points[:,2]))/(a@self.weights)

def supports(xy,target,cover=None):
 distance,_=cKDTree(xy).query(xy,k=min(9,len(xy)));s=np.maximum(distance[:,-1]*.5,32.)
 # 局所的な地形の向きに支持域を回す。格子の列・行は持たない。
 d=64.;qx=xy+[d,0];mx=xy-[d,0];qy=xy+[0,d];my=xy-[0,d];gx=(target(qx)-target(mx))/(2*d);gy=(target(qy)-target(my))/(2*d);angle=np.arctan2(gy,gx);angle=np.nan_to_num(angle)
 if cover is not None:
  # 点を格子へ移さず、影響範囲だけを広げて穴のない曲面にする。
  _,ids=cKDTree(xy).query(cover,k=min(8,len(xy)));delta=np.asarray(cover)[:,None,:]-xy[ids];co=np.cos(angle[ids]);si=np.sin(angle[ids]);u=co*delta[:,:,0]+si*delta[:,:,1];v=-si*delta[:,:,0]+co*delta[:,:,1];needed=np.maximum(abs(u),abs(v))/(1.7*s[ids]);chosen=np.argmin(needed,axis=1);rows=np.arange(len(ids));factor=np.ones(len(xy));np.maximum.at(factor,ids[rows,chosen],np.maximum(1,needed[rows,chosen]));s*=factor
 return np.column_stack((s,s)),angle

def encode(model):
 p=model.points;blob=encode_quant(p,xy=.0625,zstep=.125);restored=decode_quant(blob);_,order=cKDTree(p[:,:2]).query(restored[:,:2])
 if len(np.unique(order))!=len(p):raise ValueError('量子化後の点が重複しています')
 scale=np.rint(np.log2(model.scales[order])*256).astype('<i2');angle=np.rint((model.angles[order]%(2*np.pi))/(2*np.pi)*256).astype('u1');weight=np.rint((np.log2(model.weights[order])+2)*32).clip(0,128).astype('u1')
 raw=scale.view('u1').reshape(len(p),4).T.copy().tobytes()+angle.tobytes()+weight.tobytes();extra=zlib.compress(raw,6)
 return struct.pack('<4sIII',b'FRB1',len(p),len(blob),len(extra))+blob+extra

def decode(blob):
 magic,n,a,b=struct.unpack_from('<4sIII',blob)
 if magic!=b'FRB1' or len(blob)!=16+a+b:raise ValueError('自由Bスプラインの保存形式が不正です')
 p=decode_quant(blob[16:16+a]);raw=zlib.decompress(blob[16+a:])
 if len(p)!=n or len(raw)!=6*n:raise ValueError('自由Bスプラインの点数が不一致です')
 s=np.frombuffer(raw[:4*n],'u1').reshape(4,n).T.copy().view('<i2').reshape(n,2);angles=np.frombuffer(raw[4*n:5*n],'u1').astype(float)/256*2*np.pi;w=np.frombuffer(raw[5*n:],'u1')
 if np.any(w>128):raise ValueError('保存重みが範囲外です')
 return FreeRationalSpline(p,2**(s.astype(float)/256),angles,2**(w.astype(float)/32-2))
