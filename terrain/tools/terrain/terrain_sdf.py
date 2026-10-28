"""元DEMの三角形面から狭帯域の符号付き距離テクスチャを生成する比較。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
from pathlib import Path
import json,struct,time,zlib,lzma,io
import numpy as np
from scipy.spatial import cKDTree
from scipy.ndimage import map_coordinates
from PIL import Image
from raw_dem_reference import RawDem,center_for

OUT=Path('build/terrain-sdf-v1');HALF=8000.;BAND=256.;SCALE=1/128
CORNERS=np.array([[0,0,0],[1,0,0],[1,1,0],[0,1,0],[0,0,1],[1,0,1],[1,1,1],[0,1,1]])
TETS=[[0,1,2,6],[0,2,3,6],[0,3,7,6],[0,7,4,6],[0,4,5,6],[0,5,1,6]]
EDGES=np.array([[0,1],[0,2],[0,3],[1,2],[1,3],[2,3]])

def triangle_distance(q,a,b,c):
 u=b-a;v=c-a;w=q-a;uu=np.sum(u*u,axis=1);uv=np.sum(u*v,axis=1);vv=np.sum(v*v,axis=1);wu=np.sum(w*u,axis=1);wv=np.sum(w*v,axis=1);den=uu*vv-uv*uv
 s=(wu*vv-wv*uv)/np.maximum(den,1e-20);t=(wv*uu-wu*uv)/np.maximum(den,1e-20)
 projected=a+s[:,None]*u+t[:,None]*v
 best=np.where((s>=0)&(t>=0)&(s+t<=1),np.sum((q-projected)**2,axis=1),np.inf)
 for x,y in [(a,b),(b,c),(c,a)]:
  edge=y-x;fraction=np.clip(np.sum((q-x)*edge,axis=1)/np.maximum(np.sum(edge*edge,axis=1),1e-20),0,1)
  best=np.minimum(best,np.sum((q-x-fraction[:,None]*edge)**2,axis=1))
 return np.sqrt(best)

class SourceSurface:
 def __init__(self,dem):
  self.dem=dem;axis=np.linspace(-HALF-64,HALF+64,1025);self.axis=axis;self.step=axis[1]-axis[0]
  x,y=np.meshgrid(axis,axis);self.h=dem.sample(np.column_stack((x.ravel(),y.ravel()))).reshape(x.shape)
  if not np.isfinite(self.h).all():raise ValueError('元DEMに欠損があります')
  self.p=np.column_stack((x.ravel(),y.ravel(),self.h.ravel()));self.tree=cKDTree(self.p)
 def distance(self,q):
  _,ids=self.tree.query(q);j,i=np.divmod(ids,1025);best=np.full(len(q),np.inf)
  for dx,dy in [(-1,-1),(0,-1),(-1,0),(0,0)]:
   u=np.clip(i+dx,0,1023);v=np.clip(j+dy,0,1023);a=v*1025+u;b=a+1;c=a+1025;d=c+1
   for ia,ib,ic in [(a,b,d),(a,d,c)]:best=np.minimum(best,triangle_distance(q,self.p[ia],self.p[ib],self.p[ic]))
  return best

def case_triangles(tet,case):
 p=CORNERS[tet].astype(float);sign=np.array([1 if case&(1<<i) else -1 for i in range(4)])
 crossed=np.flatnonzero(sign[EDGES[:,0]]!=sign[EDGES[:,1]])
 if len(crossed)<3:return []
 mids=p[EDGES[crossed]].mean(axis=1);center=mids.mean(axis=0);normal=np.linalg.solve((p[1:]-p[0]),sign[1:]-sign[0]);normal/=np.linalg.norm(normal)
 u=mids[0]-center;u/=np.linalg.norm(u);v=np.cross(normal,u);delta=mids-center;order=np.argsort(np.arctan2(delta@v,delta@u));cycle=crossed[order]
 return [[cycle[0],cycle[k],cycle[k+1]] for k in range(1,len(cycle)-1)]

def extract(volume,axis,zaxis):
 # 混合セルだけにMarching Tetrahedraを適用。三角形の向きは外向きにそろえる。
 nz,ny,nx=volume.shape;lo=np.full((nz-1,ny-1,nx-1),np.inf);hi=-lo
 for dx,dy,dz in CORNERS:
  block=volume[dz:dz+nz-1,dy:dy+ny-1,dx:dx+nx-1];lo=np.minimum(lo,block);hi=np.maximum(hi,block)
 k,j,i=np.nonzero((lo<=0)&(hi>0));cells=np.column_stack((i,j,k));values=np.column_stack([volume[k+dz,j+dy,i+dx] for dx,dy,dz in CORNERS]);parts=[]
 for tet in TETS:
  tvalues=values[:,tet];cases=np.sum((tvalues>0)*np.array([1,2,4,8]),axis=1)
  for case in range(1,15):
   ids=np.flatnonzero(cases==case)
   if not len(ids):continue
   for face in case_triangles(tet,case):
    vertices=[]
    for edge in face:
     a,b=EDGES[edge];ca,cb=CORNERS[tet[a]],CORNERS[tet[b]];va=tvalues[ids,a];vb=tvalues[ids,b];fraction=va/(va-vb)
     index=cells[ids]+ca+fraction[:,None]*(cb-ca);vertices.append(np.column_stack((axis[0]+index[:,0]*(axis[1]-axis[0]),axis[0]+index[:,1]*(axis[1]-axis[0]),zaxis[0]+index[:,2]*(zaxis[1]-zaxis[0]))))
    parts.append(np.stack(vertices,axis=1))
 raw=np.concatenate(parts).reshape(-1,3);points,inverse=np.unique(np.round(raw,5),axis=0,return_inverse=True);faces=inverse.reshape(-1,3)
 keep=(faces[:,0]!=faces[:,1])&(faces[:,1]!=faces[:,2])&(faces[:,2]!=faces[:,0])
 faces=faces[keep]
 return points,faces

def png_bytes(a):
 buf=io.BytesIO();Image.fromarray(a).save(buf,format='PNG',compress_level=9);return buf.getvalue()

def generate(n,nz,source):
 started=time.perf_counter();axis=np.linspace(-HALF,HALF,n);zaxis=np.linspace(0,4000,nz);x,y=np.meshgrid(axis,axis);xy=np.column_stack((x.ravel(),y.ravel()));height=source.dem.sample(xy);volume=np.empty((nz,n,n),np.float32)
 for k,z in enumerate(zaxis):
  signed=z-height;result=np.full(len(xy),BAND);near=abs(signed)<BAND*3
  q=np.column_stack((xy[near],np.full(near.sum(),z)));result[near]=np.minimum(source.distance(q),BAND);volume[k]=(result*np.where(signed>=0,1,-1)).reshape(n,n)
 quant=np.clip(np.rint(volume/SCALE),-32768,32767).astype('<i2');decoded=quant.astype(np.float32)*SCALE
 header=dict(format='SDF1',region='yari',shape=[nz,n,n],xRange=[-HALF,HALF],yRange=[-HALF,HALF],zRange=[0,4000],distanceStep=SCALE,band=BAND,sign='positive-above',approximation='nearest-source-vertex incident triangles; vertical search band 3x',sourceGrid=1025)
 meta=json.dumps(header).encode();raw=quant.tobytes();shuffle=quant.view('u1').reshape(-1,2).T.copy().tobytes();packed=zlib.compress(shuffle,9);xz=lzma.compress(shuffle,preset=6)
 restored=np.frombuffer(zlib.decompress(packed),'u1').reshape(2,-1).T.copy().view('<i2').reshape(quant.shape)
 np.testing.assert_array_equal(restored,quant);decoded=restored.astype(np.float32)*SCALE
 columns=int(np.ceil(np.sqrt(nz)));rows=int(np.ceil(nz/columns));atlas=np.full((rows*n,columns*n),65535,dtype=np.uint16)
 for k in range(nz):atlas[(k//columns)*n:(k//columns+1)*n,(k%columns)*n:(k%columns+1)*n]=(quant[k].astype(np.int32)+32768).astype(np.uint16)
 png=png_bytes(atlas);height_png=png_bytes(np.rint(np.clip(height.reshape(n,n)*16,0,65535)).astype(np.uint16))
 OUT.mkdir(parents=True,exist_ok=True);(OUT/f'yari-{n}.sdf').write_bytes(struct.pack('<I',len(meta))+meta+packed);(OUT/f'yari-{n}.png').write_bytes(png);(OUT/f'yari-{n}-height-reference.png').write_bytes(height_png)
 p,faces=extract(decoded,axis,zaxis);truth=source.dem.sample(p[:,:2]);error=p[:,2]-truth
 # SDFの勾配を法線へ使い、抽出三角形の向きによる陰影の模様を避ける。
 gradients=np.gradient(decoded,zaxis[1]-zaxis[0],axis[1]-axis[0],axis[1]-axis[0]);coords=np.array([(p[:,2]-zaxis[0])/(zaxis[1]-zaxis[0]),(p[:,1]-axis[0])/(axis[1]-axis[0]),(p[:,0]-axis[0])/(axis[1]-axis[0])]);normal=np.column_stack([map_coordinates(gradients[c],coords,order=1,mode='nearest',prefilter=False) for c in [2,1,0]]);normal/=np.maximum(np.linalg.norm(normal,axis=1)[:,None],1e-12)
 true_grad=[]
 for c in [0,1]:
  a=p[:,:2].copy();b=a.copy();a[:,c]+=2;b[:,c]-=2;true_grad.append((source.dem.sample(a)-source.dem.sample(b))/4)
 true_normal=np.column_stack((-true_grad[0],-true_grad[1],np.ones(len(p))));true_normal/=np.linalg.norm(true_normal,axis=1)[:,None]
 reference=np.column_stack((p[:,:2],truth,true_normal)).astype('<f4');actual=np.column_stack((p,normal)).astype('<f4')
 report=dict(heightReferenceStep=.0625,horizontal=n,vertical=nz,xyStep=float(axis[1]-axis[0]),zStep=float(zaxis[1]-zaxis[0]),distanceStep=SCALE,band=BAND,vertices=len(p),triangles=len(faces),sdfZlibBytes=len(packed)+len(meta)+4,sdfXzBytes=len(xz)+len(meta)+4,sdfPngBytes=len(png),heightPngBytes=len(height_png),surfaceRmse=float(np.sqrt(np.mean(error**2))),surfaceMaximum=float(abs(error).max()),seconds=time.perf_counter()-started)
 h=json.dumps(report).encode();(OUT/f'yari-{n}.bin').write_bytes(struct.pack('<III',len(h),len(p),len(faces))+h+reference.tobytes()+actual.tobytes()+faces.astype('<u4').tobytes());(OUT/f'yari-{n}.json').write_text(json.dumps(report,indent=2),encoding='utf-8');print(json.dumps(report),flush=True)
 return report

if __name__=='__main__':
 dem=RawDem('build/terrain-cache',center_for('yari'),half=8200);source=SourceSurface(dem);reports=[generate(n,nz,source) for n,nz in [(64,64),(128,96),(256,128)]];(OUT/'manifest.json').write_text(json.dumps(reports),encoding='utf-8')
