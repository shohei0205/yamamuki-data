"""PNG実圧縮容量をそろえ、ハイトマップと距離場の復元を比較する。"""
import json,struct,io
import numpy as np
from PIL import Image
from scipy.interpolate import RectBivariateSpline
from scipy.ndimage import map_coordinates
from terrain_sdf import OUT,HALF,png_bytes
from raw_dem_reference import RawDem,center_for
from loop_dem_fit import sample_faces,metrics

def make(height_n=None,sdf_key="error400",output="capacity-compare"):
 dem=RawDem('build/terrain-cache',center_for('yari'),half=8200);sdf=json.loads((OUT/f'yari-{sdf_key}.json').read_text());target=sdf['sdfPngBytes'];best=None
 for n in ([height_n] if height_n else range(24,129)):
  axis=np.linspace(-HALF,HALF,n);x,y=np.meshgrid(axis,axis);h=dem.sample(np.column_stack((x.ravel(),y.ravel()))).reshape(n,n);png=png_bytes(np.rint(h*16).astype(np.uint16))
  if len(png)<=target and (best is None or n>best[0]):best=(n,png)
 n,png=best;(OUT/f'{output}-height.png').write_bytes(png);decoded=np.asarray(Image.open(io.BytesIO(png)),dtype=float)/16.;axis=np.linspace(-HALF,HALF,n);cubic=RectBivariateSpline(axis,axis,decoded,kx=3,ky=3,s=0)
 def evaluate(q,mode):
  coords=(q+HALF)/(2*HALF)*(n-1)
  return map_coordinates(decoded,coords[:,::-1].T,order=1,prefilter=False,mode='nearest') if mode==0 else cubic.ev(q[:,1],q[:,0])
 binary=(OUT/f'yari-{sdf_key}.bin').read_bytes();size,nv,nt=struct.unpack('<III',binary[:12]);offset=12+size+nv*24;actual=np.frombuffer(binary,dtype='<f4',count=nv*6,offset=offset).reshape(nv,6);faces=np.frombuffer(binary,dtype='<u4',offset=offset+nv*24).reshape(nt,3)
 # 保存容量とは別の共通描画格子を十分細かく置き、表示側の粗さをそろえる。
 grid=np.linspace(-7999,7999,257);gx,gy=np.meshgrid(grid,grid);drawxy=np.column_stack((gx.ravel(),gy.ravel()))
 sdf_values=np.column_stack([sample_faces(actual[:,:2],faces,actual[:,c],drawxy) for c in [2,3,4,5]])
 sdf_values[:,1:]/=np.linalg.norm(sdf_values[:,1:],axis=1)[:,None]
 actual=np.column_stack((drawxy,sdf_values)).astype('<f4');cell=np.arange(256)[:,None]*257+np.arange(256)[None,:];a=cell.ravel();faces=np.concatenate((np.column_stack((a,a+1,a+258)),np.column_stack((a,a+258,a+257)))).astype('<u4');nv=len(actual);nt=len(faces)
 arrays=[]
 for mode in [0,1]:
  q=actual[:,:2].astype(float);h=evaluate(q,mode);g=[]
  for c in [0,1]:
   a=q.copy();b=q.copy();a[:,c]=np.minimum(HALF,a[:,c]+2);b[:,c]=np.maximum(-HALF,b[:,c]-2);g.append((evaluate(a,mode)-evaluate(b,mode))/(a[:,c]-b[:,c]))
  normal=np.column_stack((-g[0],-g[1],np.ones(len(q))));normal/=np.linalg.norm(normal,axis=1)[:,None];arrays.append(np.column_stack((q,h,normal)).astype('<f4'))
 arrays.append(actual)
 test=np.linspace(-7900,7900,257);x,y=np.meshgrid(test,test);q=np.column_stack((x.ravel(),y.ravel()));truth=dem.sample(q)
 # 実際の描画メッシュに対し、両方式を同じ検査点で評価する。
 audits=[metrics(sample_faces(a[:,:2],faces,a[:,2],q),truth) for a in arrays]
 report=dict(heightResolution=n,heightPngBytes=len(png),sdfPngBytes=target,ratio=len(png)/target,heightStep=.0625,vertices=nv,triangles=nt,audits=audits,labels=['ハイトマップ・バイリニア','ハイトマップ・三次スプライン','距離場'],domain=16000,sdfShape=[sdf["horizontal"],sdf["horizontal"],sdf["vertical"]],sameResolution=height_n is not None,testSamples=len(q),sharedTopology=True,drawGrid=257,lpf=False)
 header=json.dumps(report,ensure_ascii=False).encode();(OUT/f'{output}.bin').write_bytes(struct.pack('<III',len(header),nv,nt)+header+b''.join(a.astype('<f4').tobytes() for a in arrays)+faces.astype('<u4').tobytes());(OUT/f'{output}.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8');print(json.dumps(report,ensure_ascii=False))
if __name__=='__main__':make()
