"""槍ヶ岳の元DEMへ、Loop細分割後の制御点の高さを直接適合する比較。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json,struct,time,zlib
from pathlib import Path
import numpy as np
from scipy import sparse
from scipy.spatial import Delaunay,cKDTree
from scipy.interpolate import LinearNDInterpolator
from scipy.sparse.linalg import lsmr
from scipy.stats import qmc
from raw_dem_reference import RawDem,center_for

OUT=Path('build/loop-dem-fit-v1')

def loop_operator(n,faces):
 faces=np.asarray(faces,dtype=np.int64)
 edges=np.concatenate((faces[:,[0,1]],faces[:,[1,2]],faces[:,[2,0]]));opposite=np.concatenate((faces[:,2],faces[:,0],faces[:,1]))
 edges.sort(axis=1);unique,inverse,counts=np.unique(edges,axis=0,return_inverse=True,return_counts=True)
 if counts.max()>2:raise ValueError('非多様体メッシュ')
 boundary=np.unique(unique[counts==1]);degree=np.bincount(unique.ravel(),minlength=n)
 beta=(5/8-(3/8+.25*np.cos(2*np.pi/np.maximum(degree,1)))**2)/np.maximum(degree,1);beta[boundary]=0
 r=[np.arange(n),unique[:,0],unique[:,1]];c=[np.arange(n),unique[:,1],unique[:,0]];w=[1-degree*beta,beta[unique[:,0]],beta[unique[:,1]]]
 idx=n+np.arange(len(unique));edge_weight=np.where(counts==2,.375,.5)
 for k in [0,1]:r.append(idx);c.append(unique[:,k]);w.append(edge_weight)
 internal=counts[inverse]==2;r.append(n+inverse[internal]);c.append(opposite[internal]);w.append(np.full(internal.sum(),.125))
 matrix=sparse.csr_matrix((np.concatenate(w),(np.concatenate(r),np.concatenate(c))),shape=(n+len(unique),n))
 ab,bc,ca=(n+inverse).reshape(3,-1);a,b,c0=faces.T
 refined=np.concatenate((np.column_stack((a,ab,ca)),np.column_stack((b,bc,ab)),np.column_stack((c0,ca,bc)),np.column_stack((ab,bc,ca))))
 return matrix,refined,boundary

def triangle_angles(xy,faces):
 f=xy[faces];angles=[]
 for i in range(3):
  a=f[:,(i+1)%3]-f[:,i];b=f[:,(i+2)%3]-f[:,i]
  cosine=np.sum(a*b,axis=1)/np.maximum(np.linalg.norm(a,axis=1)*np.linalg.norm(b,axis=1),1e-20)
  angles.append(np.degrees(np.arccos(np.clip(cosine,-1,1))))
 return np.min(angles,axis=0)

def initial_mesh(dem,budget):
 edge=np.linspace(-10000,10000,17)
 xy=np.unique(np.concatenate((np.column_stack((edge,np.full(17,-10000))),np.column_stack((edge,np.full(17,10000))),np.column_stack((np.full(17,-10000),edge)),np.column_stack((np.full(17,10000),edge)),qmc.Sobol(2,scramble=True,seed=34).random_base2(6)*20000-10000)),axis=0)
 axis=np.linspace(-9800,9800,257);u,v=np.meshgrid(axis,axis);query=np.column_stack((u.ravel(),v.ravel()));truth=dem.sample(query);valid=np.isfinite(truth);query=query[valid];truth=truth[valid]
 while len(xy)<budget:
  z=dem.sample(xy)
  if not np.isfinite(z).all():raise ValueError('制御領域に欠損DEMがあります')
  tri=Delaunay(xy);pred=LinearNDInterpolator(tri,z)(query);error=abs(pred-truth)
  tree=cKDTree(xy);quality=[]
  # 鋭角面の外心に点を足し、細長い面を改善する枠を誤差点と競合させる。
  faces=tri.simplices;angles=triangle_angles(xy,faces)
  for face in faces[np.argsort(angles)[:16]]:
   a,b,c=xy[face];matrix=2*np.array([b-a,c-a]);rhs=np.array([b@b-a@a,c@c-a@a])
   if abs(np.linalg.det(matrix))<1e-8:continue
   center=np.linalg.solve(matrix,rhs)
   if np.max(abs(center))<9900 and tree.query(center)[0]>80 and all(np.linalg.norm(center-other)>120 for other in quality):quality.append(center)
   if len(quality)>=min(8,budget-len(xy)):break
  if quality:xy=np.concatenate((xy,np.asarray(quality)));tree=cKDTree(xy)
  if len(xy)>=budget:break
  order=np.argsort(-error);distance=tree.query(query)[0];ids=[]
  for i in order:
   if distance[i]<25:continue
   if all(np.linalg.norm(query[i]-query[j])>120 for j in ids):ids.append(i)
   if len(ids)>=min(32,budget-len(xy)):break
  if not ids:break
  xy=np.concatenate((xy,query[ids]));print('control',len(xy),flush=True)
 return np.column_stack((xy,dem.sample(xy))),Delaunay(xy).simplices

def normals(p,faces):
 f=p[faces];face=np.cross(f[:,1]-f[:,0],f[:,2]-f[:,0]);face[face[:,2]<0]*=-1;n=np.zeros_like(p)
 for k in range(3):np.add.at(n,faces[:,k],face)
 return n/np.maximum(np.linalg.norm(n,axis=1)[:,None],1e-12)

def sample_faces(xy,faces,h,q):
 # 再Delaunay化せず、実際に描画する面の重心近傍から点を含む面を探す。
 f=xy[faces];tree=cKDTree(f.mean(axis=1));out=np.full(len(q),np.nan)
 for count in [32,128,512]:
  missing=np.flatnonzero(~np.isfinite(out))
  if not len(missing):break
  _,ids=tree.query(q[missing],k=min(count,len(f)));a=f[ids,0];u=f[ids,1]-a;v=f[ids,2]-a;d=q[missing,None,:]-a
  det=u[:,:,0]*v[:,:,1]-u[:,:,1]*v[:,:,0];det=np.where(abs(det)>1e-12,det,np.nan)
  b=(d[:,:,0]*v[:,:,1]-d[:,:,1]*v[:,:,0])/det;c=(u[:,:,0]*d[:,:,1]-u[:,:,1]*d[:,:,0])/det
  valid=(b>=-1e-9)&(c>=-1e-9)&(b+c<=1+1e-9);found=valid.any(axis=1);row=np.flatnonzero(found);col=valid.argmax(axis=1)[found];face=faces[ids[row,col]]
  out[missing[found]]=h[face[:,0]]*(1-b[row,col]-c[row,col])+h[face[:,1]]*b[row,col]+h[face[:,2]]*c[row,col]
 # 細長い面では重心が遠いため、近傍候補で見つからない点だけ面の範囲で再探索する。
 for i in np.flatnonzero(~np.isfinite(out)):
  ids=np.flatnonzero(np.all(f.min(axis=1)<=q[i]+1e-6,axis=1)&np.all(f.max(axis=1)>=q[i]-1e-6,axis=1))
  a=f[ids,0];u=f[ids,1]-a;v=f[ids,2]-a;d=q[i]-a;det=u[:,0]*v[:,1]-u[:,1]*v[:,0];det=np.where(abs(det)>1e-12,det,np.nan)
  b=(d[:,0]*v[:,1]-d[:,1]*v[:,0])/det;c=(u[:,0]*d[:,1]-u[:,1]*d[:,0])/det;valid=np.flatnonzero((b>=-1e-6)&(c>=-1e-6)&(b+c<=1+1e-6))
  if len(valid):
   k=valid[0];face=faces[ids[k]];out[i]=h[face[0]]*(1-b[k]-c[k])+h[face[1]]*b[k]+h[face[2]]*c[k]
 if not np.isfinite(out).all():raise ValueError('評価点を含む描画面が見つかりません')
 return out

def metrics(pred,truth):
 valid=np.isfinite(truth);e=pred[valid]-truth[valid];return dict(samples=int(valid.sum()),rmse=float(np.sqrt(np.mean(e*e))),maximum=float(abs(e).max()))

def generate(budget,dem):
 started=time.perf_counter();p,faces=initial_mesh(dem,budget);base_faces=faces.copy();A=sparse.eye(len(p),format='csr')
 for _ in range(3):
  B,faces,boundary=loop_operator(A.shape[0],faces);A=B@A
 xy=A@p[:,:2];original=A@p[:,2];truth=dem.sample(xy);valid=np.isfinite(truth)
 # 面積で重み付けし、細かい面の密集箇所だけが適合を支配するのを避ける。
 f=xy[faces];delta=f[:,1:]-f[:,:1];area=abs(delta[:,0,0]*delta[:,1,1]-delta[:,0,1]*delta[:,1,0])*.5;weights=np.zeros(len(xy))
 for k in range(3):np.add.at(weights,faces[:,k],area/3)
 weight=np.sqrt(weights[valid]/np.mean(weights[valid]));design=A[valid].multiply(weight[:,None]);residual=(truth[valid]-original[valid])*weight
 solution=lsmr(design,residual,damp=.3,atol=1e-7,btol=1e-7,maxiter=250);z=p[:,2]+solution[0];fitted=A@z
 # 元DEMから別に抽出したランダム点で評価する。適合に使った曲面頂点とは分離。
 q=(qmc.Sobol(2,scramble=True,seed=45).random_base2(14)*19000-9500);target=dem.sample(q)
 angles=triangle_angles(p[:,:2],base_faces)
 report=dict(minimumControlAngleDegrees=float(angles.min()),controlFacesBelow10Degrees=int((angles<10).sum()),controlVertices=len(p),controlTriangles=len(base_faces),drawVertices=len(xy),drawTriangles=len(faces),subdivisions=3,fitBefore=metrics(original,truth),fitAfter=metrics(fitted,truth),holdoutBefore=metrics(sample_faces(xy,faces,original,q),target),holdoutAfter=metrics(sample_faces(xy,faces,fitted,q),target),iterations=int(solution[2]),maxControlShift=float(abs(solution[0]).max()),source='元DEM・LPFなし',seconds=time.perf_counter()-started)
 vertices=[]
 for h in [original,fitted]:
  pos=np.column_stack((xy,h));vertices.append(np.column_stack((pos,normals(pos,faces))).astype('<f4'))
 # 比較描画用の展開済み配列。配布時の必要量とは区別する。
 control=np.column_stack((p[:,:2],z)).astype('<f4');raw=control.tobytes()+np.asarray(base_faces,dtype='<u4').tobytes();report['controlFloat32ZlibBytes']=len(zlib.compress(raw,6));report['controlRawBytes']=len(raw)
 header=json.dumps(report,ensure_ascii=False).encode();body=struct.pack('<III',len(header),len(xy),len(faces))+header+vertices[0].tobytes()+vertices[1].tobytes()+np.asarray(faces,dtype='<u4').tobytes()
 OUT.mkdir(parents=True,exist_ok=True);(OUT/f'yari-{budget}.bin').write_bytes(body);(OUT/f'yari-{budget}-control.bin').write_bytes(raw);(OUT/f'yari-{budget}.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8');print(json.dumps(report),flush=True)
 return report

if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True);dem=RawDem('build/terrain-cache',center_for('yari'),half=10500)
 reports=[generate(b,dem) for b in [512,2048,8192]];(OUT/'manifest.json').write_text(json.dumps(reports),encoding='utf-8')
