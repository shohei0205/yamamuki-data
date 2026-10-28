"""Sibson曲面を4点四角形・16係数Bezier・3頂点傾き曲面へ近似する比較。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import base64,gzip,json,struct,time,zlib,urllib.request
from pathlib import Path
import numpy as np
from scipy.spatial import Delaunay,ConvexHull
from scipy.interpolate import LinearNDInterpolator
from sibson_surface import SibsonSurface
from sibson_tile_seams import extract_points
OUT=Path('build/control-patch-comparison-v1')

def basis(t,derivative=0):
 if derivative==0:return np.array([2*t**3-3*t*t+1,-2*t**3+3*t*t]),np.array([t**3-2*t*t+t,t**3-t*t])
 return np.array([6*t*t-6*t,-6*t*t+6*t]),np.array([3*t*t-4*t+1,3*t*t-2*t])

def quad_eval(q,rect,features,cubic,dx=0,dy=0):
 width=rect[2]-rect[0];height=rect[3]-rect[1];u=(q[:,0]-rect[0])/width;v=(q[:,1]-rect[1])/height
 if not cubic:
  ux=np.array([1-u,u]) if not dx else np.array([-np.ones(len(u)),np.ones(len(u))])/width
  vy=np.array([1-v,v]) if not dy else np.array([-np.ones(len(v)),np.ones(len(v))])/height
  return sum(features[i,j,0]*ux[i]*vy[j] for i in range(2) for j in range(2))
 U,V=basis(u,dx);Y,Z=basis(v,dy);result=np.zeros(len(q))
 for i in range(2):
  for j in range(2):
   h,hx,hy,hxy=features[i,j];result+=h*U[i]*Y[j]+hx*width*V[i]*Y[j]+hy*height*U[i]*Z[j]+hxy*width*height*V[i]*Z[j]
 return result/width**dx/height**dy

def features_at(model,q):
 h=model(q);g=[]
 for axis in [0,1]:
  a=q.copy();b=q.copy();a[:,axis]+=1;b[:,axis]-=1;g.append((model(a)-model(b))/2)
 # 混合微分はゼロの初期値。接合の拘束時には隣の曲面の値から決める。
 return np.column_stack((h,*g,np.zeros(len(q))))

def quads(model,error):
 cache={}
 def feature(x,y):
  key=(float(x),float(y))
  if key not in cache:cache[key]=features_at(model,np.array([key]))[0]
  return cache[key].copy()
 def data(rect):
  a,b,c,d=rect;return np.array([[feature(a,b),feature(a,d)],[feature(c,b),feature(c,d)]])
 leaves=[];pending=[(-8000.,-8000.,8000.,8000.,0)]
 while pending:
  a,b,c,d,depth=pending.pop();f=data((a,b,c,d));x,y=np.meshgrid(np.linspace(a,c,5),np.linspace(b,d,5));q=np.column_stack((x.ravel(),y.ravel()));difference=np.max(abs(model(q)-quad_eval(q,(a,b,c,d),f,False)))
  if difference>error and depth<5:
   mx=(a+c)/2;my=(b+d)/2;pending.extend([(a,b,mx,my,depth+1),(mx,b,c,my,depth+1),(a,my,mx,d,depth+1),(mx,my,c,d,depth+1)])
  else:leaves.append((np.array([a,b,c,d]),f))
 leaves.sort(key=lambda item:-(item[0][2]-item[0][0]));result=[]
 # 粗いパッチの辺上にある細かい側の角を、粗い曲面の値と傾きへ拘束する。
 for cubic in [False,True]:
  fitted=[];shared={}
  for rect,raw in leaves:
   f=raw.copy()
   for i,x in enumerate([rect[0],rect[2]]):
    for j,y in enumerate([rect[1],rect[3]]):
     key=(x,y)
     if key in shared:f[i,j]=shared[key];continue
     q=np.array([[x,y]])
     for coarse,cf in fitted:
      if coarse[0]-1e-7<=x<=coarse[2]+1e-7 and coarse[1]-1e-7<=y<=coarse[3]+1e-7 and (x in [coarse[0],coarse[2]] or y in [coarse[1],coarse[3]]):
       f[i,j,0]=quad_eval(q,coarse,cf,cubic)[0]
       if cubic:
        f[i,j,1]=quad_eval(q,coarse,cf,True,1,0)[0];f[i,j,2]=quad_eval(q,coarse,cf,True,0,1)[0];f[i,j,3]=quad_eval(q,coarse,cf,True,1,1)[0]
       break
     shared[key]=f[i,j].copy()
   fitted.append((rect,f))
  result.append(QuadSurface(fitted,cubic))
 return result

class QuadSurface:
 def __init__(self,patches,cubic):self.patches=patches;self.cubic=cubic
 def __call__(self,q):
  out=np.full(len(q),np.nan)
  for rect,f in self.patches:
   ids=np.flatnonzero((q[:,0]>=rect[0]-1e-8)&(q[:,0]<=rect[2]+1e-8)&(q[:,1]>=rect[1]-1e-8)&(q[:,1]<=rect[3]+1e-8))
   if len(ids):out[ids]=quad_eval(q[ids],rect,f,self.cubic)
  if not np.isfinite(out).all():raise ValueError('パッチの領域に欠損があります')
  return out
 def payload(self):
  values=[]
  for rect,f in self.patches:
   if not self.cubic:values.extend([*rect,*f[:,:,0].ravel()]);continue
   net=np.zeros((4,4));w=rect[2]-rect[0];h=rect[3]-rect[1]
   for i in range(2):
    for j in range(2):
     a=3*i;b=3*j;di=1-2*i;dj=1-2*j;z,hx,hy,hxy=f[i,j];net[a,b]=z;net[a+di,b]=z+di*hx*w/3;net[a,b+dj]=z+dj*hy*h/3;net[a+di,b+dj]=z+di*hx*w/3+dj*hy*h/3+di*dj*hxy*w*h/9
   values.extend([*rect,*net.ravel()])
  return np.asarray(values,dtype='<f4').tobytes()

class TriangleSurface:
 def __init__(self,p,gradient):
  self.p=p;self.gradient=gradient;self.tri=Delaunay(p[:,:2]);self.coefficients=[]
  for ids in self.tri.simplices:
   v=p[ids];g=gradient[ids];a,b,c=v[:,2];ab=v[1,:2]-v[0,:2];ac=v[2,:2]-v[0,:2];bc=v[2,:2]-v[1,:2]
   edge=[a+g[0]@ab/3,b-g[1]@ab/3,a+g[0]@ac/3,c-g[2]@ac/3,b+g[1]@bc/3,c-g[2]@bc/3]
   self.coefficients.append([a,b,c,*edge,sum(edge)/6])
  self.coefficients=np.asarray(self.coefficients)
 def __call__(self,q):
  ids=self.tri.find_simplex(q)
  if (ids<0).any():raise ValueError('三角形の範囲外')
  t=self.tri.transform[ids];uv=np.einsum('nij,nj->ni',t[:,:2,:],q-t[:,2,:]);u,v=uv.T;w=1-u-v;c=self.coefficients[ids]
  terms=np.column_stack((u**3,v**3,w**3,3*u*u*v,3*u*v*v,3*u*u*w,3*u*w*w,3*v*v*w,3*v*w*w,6*u*v*w));return np.sum(c*terms,axis=1)
 def payload(self):return np.column_stack((self.p,self.gradient)).astype('<f4').tobytes()+self.tri.simplices.astype('<u4').tobytes()

def triangle_fit(model,input_points,error):
 candidate=input_points[(abs(input_points[:,0])<7999)&(abs(input_points[:,1])<7999)]
 corners=np.array([[-8000.,-8000.],[-8000,8000],[8000,-8000],[8000,8000]])
 xy=corners.copy()
 for _ in range(64):
  z=model(xy);tri=Delaunay(xy);residual=abs(LinearNDInterpolator(tri,z)(candidate[:,:2])-candidate[:,2]);order=np.argsort(-residual);selected=[]
  for index in order:
   if residual[index]<=error:break
   if all(np.linalg.norm(candidate[index,:2]-candidate[j,:2])>100 for j in selected):selected.append(index)
   if len(selected)>=32:break
  if not selected:break
  xy=np.concatenate((xy,candidate[selected,:2]));xy=np.unique(xy,axis=0)
 p=np.column_stack((xy,model(xy)));f=features_at(model,xy);return TriangleSurface(p,f[:,1:3])

def input_model(region):
 with urllib.request.urlopen('http://127.0.0.1:61473/api/ridge-spline-input?region='+region,timeout=120) as response:b=response.read()
 source=json.loads(gzip.decompress(b) if b[:2]==b'\x1f\x8b' else b);p=np.frombuffer(base64.b64decode(source['points']),'<f4').reshape(-1,3).astype(float);model=SibsonSurface(p);return p,model

def run(region,error,p,model):
 saved_reference=extract_points(model,[-8000,-8000,8000,8000]);started=time.perf_counter();bilinear,bezier=quads(model,error);triangle=triangle_fit(model,p,error);methods=[model,bilinear,bezier,triangle];labels=['Sibson自然近傍','4点・適応四角形','16係数・三次Bezier','3頂点＋傾き・三角形曲面'];axis=np.linspace(-7999,7999,257);x,y=np.meshgrid(axis,axis);q=np.column_stack((x.ravel(),y.ravel()));reference=model(q);arrays=[];reports=[]
 for i,method in enumerate(methods):
  start=time.perf_counter();h=method(q);seconds=time.perf_counter()-start;g=[]
  for c in [0,1]:
   a=q.copy();b=q.copy();a[:,c]+=.5;b[:,c]-=.5;g.append(method(a)-method(b))
  normal=np.column_stack((-g[0],-g[1],np.ones(len(q))));normal/=np.linalg.norm(normal,axis=1)[:,None];arrays.append(np.column_stack((q,h,normal)).astype('<f4'));raw=saved_reference.astype('<f4').tobytes() if i==0 else method.payload();packed=zlib.compress(raw,6);(OUT/f'{region}-{int(error)}-{i}.zlib').write_bytes(packed);difference=h-reference;reports.append(dict(label=labels[i],bytes=len(packed),rawBytes=len(raw),rmse=float(np.sqrt(np.mean(difference**2))),maximum=float(abs(difference).max()),evaluationSeconds=seconds,patches=len(method.patches) if i in [1,2] else len(triangle.tri.simplices) if i==3 else None,controls=len(triangle.p) if i==3 else len(saved_reference) if i==0 else None))
 cell=(np.arange(256)[:,None]*257+np.arange(256)[None,:]).ravel();faces=np.concatenate((np.column_stack((cell,cell+1,cell+258)),np.column_stack((cell,cell+258,cell+257)))).astype('<u4');report=dict(region=region,targetError=error,methods=reports,vertices=len(q),triangles=len(faces),seconds=time.perf_counter()-started,reference='Sibson・同じ入力点',scope=16000);head=json.dumps(report,ensure_ascii=False).encode();(OUT/f'{region}-{int(error)}.bin').write_bytes(struct.pack('<III',len(head),len(q),len(faces))+head+b''.join(a.tobytes() for a in arrays)+faces.tobytes());(OUT/f'{region}-{int(error)}.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8');print(json.dumps(report,ensure_ascii=False),flush=True)

if __name__=='__main__':
 OUT.mkdir(parents=True,exist_ok=True)
 for region in ['fuji','yari','tsukuba']:
  p,model=input_model(region)
  for error in [100,400,25]:run(region,error,p,model)
