"""標準の非周期3次NURBS曲面。XYZすべてを同じ有理基底で評価する。"""
import json,struct,zlib
import numpy as np
from scipy.interpolate import BSpline,RegularGridInterpolator
from scipy.sparse import csr_matrix,diags,eye,vstack
from scipy.sparse.linalg import lsmr
from scipy.spatial import cKDTree
from scipy.stats import qmc

def knots(n):
 if n<4:raise ValueError('3次NURBSには各方向4制御点以上が必要です')
 return np.r_[np.zeros(4),np.arange(1,n-3)/(n-3),np.ones(4)]
def greville(t):return np.array([np.mean(t[i+1:i+4]) for i in range(len(t)-4)])
def basis1d(x,t,derivative=False):
 x=np.clip(np.asarray(x,float),0,1);b=BSpline.design_matrix(x,t,3).tocsr();ids=b.indices.reshape(-1,4);values=b.data.reshape(-1,4)
 if not derivative:return ids,values,None
 d=BSpline.design_matrix(np.clip(x,1e-13,1-1e-13),t,2).tocsr();di=d.indices.reshape(-1,3);dv=d.data.reshape(-1,3)
 left=np.sum(dv[:,None,:]*(di[:,None,:]==ids[:,:,None]),axis=2);right=np.sum(dv[:,None,:]*(di[:,None,:]==ids[:,:,None]+1),axis=2);a=t[ids+3]-t[ids];c=t[ids+4]-t[ids+1]
 derivatives=np.divide(3*left,a,out=np.zeros_like(left),where=a>0)-np.divide(3*right,c,out=np.zeros_like(right),where=c>0)
 return ids,values,derivatives

def design(uv,tu,tv,derivative=False):
 uv=np.asarray(uv,float);iu,u,du=basis1d(uv[:,0],tu,derivative);iv,v,dv=basis1d(uv[:,1],tv,derivative);n=len(tu)-4;m=len(tv)-4;col=(iv[:,:,None]*n+iu[:,None,:]).reshape(-1);rows=np.repeat(np.arange(len(uv)),16)
 def matrix(data):return csr_matrix((data.reshape(-1),(rows,col)),shape=(len(uv),n*m))
 a=matrix(v[:,:,None]*u[:,None,:])
 if not derivative:return a
 return a,matrix(v[:,:,None]*du[:,None,:]),matrix(dv[:,:,None]*u[:,None,:])

class NurbsSurface:
 def __init__(self,points,weights,tu,tv):
  self.points=np.asarray(points,float);self.weights=np.asarray(weights,float);self.tu=np.asarray(tu,float);self.tv=np.asarray(tv,float);n=(len(tu)-4)*(len(tv)-4)
  if self.points.shape!=(n,3) or self.weights.shape!=(n,) or not np.isfinite(self.points).all() or not np.isfinite(self.weights).all() or np.any(self.weights<=0) or np.any(np.diff(tu)<0) or np.any(np.diff(tv)<0):raise ValueError('NURBSの制御点・重み・節点が不正です')
 def evaluate(self,uv,derivative=False):
  if derivative:a,au,av=design(uv,self.tu,self.tv,True)
  else:a=design(uv,self.tu,self.tv)
  den=a@self.weights;position=(a@(self.weights[:,None]*self.points))/den[:,None]
  if not derivative:return position
  su=(au@(self.weights[:,None]*self.points)-position*(au@self.weights)[:,None])/den[:,None];sv=(av@(self.weights[:,None]*self.points)-position*(av@self.weights)[:,None])/den[:,None]
  return position,su,sv
 def inverse_xy(self,q,tolerance=1e-5):
  q=np.asarray(q,float);lo=self.points[:,:2].min(axis=0);hi=self.points[:,:2].max(axis=0);uv=np.clip((q-lo)/(hi-lo),0,1)
  for _ in range(50):
   p,u,v=self.evaluate(uv,True);e=p[:,:2]-q
   if np.max(abs(e))<tolerance:return uv
   det=u[:,0]*v[:,1]-u[:,1]*v[:,0]
   if np.any(det<=0):raise ValueError('NURBSのXY写像が反転しています')
   step=np.column_stack(((v[:,1]*e[:,0]-v[:,0]*e[:,1])/det,(-u[:,1]*e[:,0]+u[:,0]*e[:,1])/det));scale=np.maximum(1,np.max(abs(step),axis=1)/.05);step=step/scale[:,None];candidate=np.clip(uv-step,0,1);old=np.sum(e*e,axis=1);active=np.flatnonzero(old>tolerance*tolerance)
   for trial in range(14):
    error=self.evaluate(candidate[active])[:,:2]-q[active];bad=np.sum(error*error,axis=1)>old[active]
    if not bad.any():break
    active=active[bad];step[active]*=.5;candidate[active]=np.clip(uv[active]-step[active],0,1)
   uv=candidate
  if np.max(abs(self.evaluate(uv)[:,:2]-q))>=tolerance:raise ValueError('NURBSのXY逆写像が収束しません')
  return uv
 def __call__(self,q):return self.evaluate(self.inverse_xy(q))[:,2]
 def height_gradient(self,q):
  p,u,v=self.evaluate(self.inverse_xy(q),True);det=u[:,0]*v[:,1]-u[:,1]*v[:,0]
  if np.any(det<=0):raise ValueError('NURBSのXY写像が反転しています')
  return np.column_stack((p[:,2],(u[:,2]*v[:,1]-v[:,2]*u[:,1])/det,(u[:,0]*v[:,2]-v[:,0]*u[:,2])/det))

def regular_xy(n):
 t=knots(n);g=greville(t);x,y=np.meshgrid(g,g);return np.column_stack((x.ravel(),y.ravel()))*16000-8000

def adapted_xy(n,target):
 # 少数点の制御網を地形重要度に応じて曲げ、その変形を詳細な網へ引き継ぐ。
 small=17;g=greville(knots(small));x,y=np.meshgrid(g,g);base=np.column_stack((x.ravel(),y.ravel()))*16000-8000;xy=base.copy();q=(qmc.Sobol(2,scramble=True,seed=817).random_base2(16)-.5)*16000;h=target(q);d=200.;rough=abs(4*h-target(q+[d,0])-target(q-[d,0])-target(q+[0,d])-target(q-[0,d]));valid=np.isfinite(h)&np.isfinite(rough);q=q[valid];rough=rough[valid];importance=1+8*np.sqrt(np.clip(rough/max(np.percentile(rough,90),1e-6),0,4));edge=np.zeros((small,small),bool);edge[[0,-1],:]=True;edge[:,[0,-1]]=True;edge=edge.ravel()
 def positive(p):
  p=p.reshape(small,small,2);a=p[:-1,:-1];b=p[:-1,1:];c=p[1:,1:];d=p[1:,:-1]
  cross=lambda u,v:u[:,:,0]*v[:,:,1]-u[:,:,1]*v[:,:,0]
  return min(cross(b-a,d-a).min(),cross(c-b,a-b).min(),cross(d-c,b-c).min(),cross(a-d,c-d).min())>100
 from scipy.ndimage import gaussian_filter
 for iteration in range(80):
  _,ids=cKDTree(xy).query(q);mass=np.bincount(ids,weights=importance,minlength=len(xy));cx=np.bincount(ids,weights=importance*q[:,0],minlength=len(xy));cy=np.bincount(ids,weights=importance*q[:,1],minlength=len(xy));centroid=np.column_stack((cx,cy))/np.maximum(mass[:,None],1e-12);move=(centroid-xy)*.5;move[mass==0]=0;move=gaussian_filter(move.reshape(small,small,2),sigma=(.6,.6,0)).reshape(-1,2);move[edge]=0;length=np.linalg.norm(move,axis=1);move/=np.maximum(1,length[:,None]/200)
  for rate in [1,.5,.25,.125]:
   candidate=xy+move*rate
   if positive(candidate):xy=candidate;break
 displacement=(xy-base).reshape(small,small,2);t=knots(n);ng=greville(t);a,b=np.meshgrid(ng,ng);warp=RegularGridInterpolator((g,g),displacement)(np.column_stack((b.ravel(),a.ravel())));return regular_xy(n)+warp

def fit_nurbs(xy,n,target,error,weighted=True,iterations=10,initial=None,progress=None):
 tu=knots(n) if initial is None else initial.tu;tv=knots(n) if initial is None else initial.tv;power=max(15,int(np.ceil(np.log2(n*n*16))));uv=qmc.Sobol(2,scramble=True,seed=194).random_base2(power);a=design(uv,tu,tv);z0=target(xy) if initial is None else initial.points[:,2].copy();z=z0.copy();logw=np.zeros(len(xy)) if initial is None else np.log(initial.weights);history=[]
 base=NurbsSurface(np.column_stack((xy,z)),np.exp(logw),tu,tv);_,u,v=base.evaluate(uv,True);det=u[:,0]*v[:,1]-u[:,1]*v[:,0]
 if np.any(det<=0):raise ValueError('変形した制御網が反転しています')
 sample_weight=np.sqrt(det/np.mean(det));reg=.0003
 def parts(l):
  w=np.exp(l);b=diags(1/(a@w))@a@diags(w);q=b@xy;truth=target(q);return b,q,truth
 def objective(z,l):
  b,q,truth=parts(l);r=(b@z-truth)*sample_weight/error;return float(r@r+reg*np.sum(((z-z0)/error)**2)+.1*np.sum(l*l))
 for iteration in range(iterations if weighted else 1):
  b,q,truth=parts(logw);valid=np.isfinite(truth);bw=diags(sample_weight[valid])@b[valid];delta=lsmr(vstack((bw,eye(len(z))*np.sqrt(reg))),np.r_[(truth[valid]-b[valid]@z0)*sample_weight[valid],np.zeros(len(z))],atol=1e-6,btol=1e-6,maxiter=250)[0];z=z0+delta;before=objective(z,logw);accepted=False
  if weighted:
   gx=(target(q+[8.,0])-target(q-[8.,0]))/16;gy=(target(q+[0,8.])-target(q-[0,8.]))/16;prediction=b@z
   j=b@diags(z)-diags(gx)@b@diags(xy[:,0])-diags(gy)@b@diags(xy[:,1])-diags(prediction-gx*q[:,0]-gy*q[:,1])@b;j=diags(sample_weight/error)@j
   step=lsmr(vstack((j[valid],eye(len(z))*np.sqrt(.1))),np.r_[(truth[valid]-prediction[valid])*sample_weight[valid]/error,-np.sqrt(.1)*logw],atol=1e-5,btol=1e-5,maxiter=100)[0]
   for rate in [1.,.5,.25,.125]:
    candidate=np.clip(logw+rate*step,np.log(.5),np.log(2.))
    if objective(z,candidate)<before:logw=candidate;accepted=True;break
  history.append(dict(iteration=iteration,objective=objective(z,logw),accepted=accepted))
  if progress is not None:progress(history[-1])
  if weighted and not accepted:break
 return NurbsSurface(np.column_stack((xy,z)),np.exp(logw),tu,tv),dict(trainingSamples=len(uv),history=history,minimumInitialJacobian=float(det.min()))

def encode(model):
 n=len(model.tu)-4;m=len(model.tv)-4;p=np.rint(model.points/[.0625,.0625,.125]).astype('<i4');delta=np.diff(p,axis=0,prepend=np.zeros((1,3),dtype='<i4')).astype('<i4');w=np.rint((np.log2(model.weights)+2)*32).astype('u1');raw=delta.view('u1').reshape(-1,12).T.copy().tobytes()+w.tobytes()+model.tu.astype('<f8').tobytes()+model.tv.astype('<f8').tobytes();packed=zlib.compress(raw,6);return struct.pack('<4sIII',b'NRB1',n,m,len(packed))+packed

def decode(blob):
 magic,n,m,length=struct.unpack_from('<4sIII',blob)
 if magic!=b'NRB1' or len(blob)!=16+length or min(n,m)<4:raise ValueError('NURBSの保存形式が不正です')
 raw=zlib.decompress(blob[16:]);count=n*m
 if len(raw)!=13*count+8*(n+m+8):raise ValueError('NURBSの保存長が不正です')
 delta=np.frombuffer(raw[:12*count],'u1').reshape(12,count).T.copy().view('<i4').reshape(count,3);p=np.cumsum(delta.astype(np.int64),axis=0)*[.0625,.0625,.125];w=np.frombuffer(raw[12*count:13*count],'u1')
 if np.any(w>128):raise ValueError('NURBSの保存重みが不正です')
 tu=np.frombuffer(raw,'<f8',n+4,13*count).copy();tv=np.frombuffer(raw,'<f8',m+4,13*count+8*(n+4)).copy();return NurbsSurface(p,2**(w.astype(float)/32-2),tu,tv)


def refine_to(model,n):
 """同次座標の節点挿入で、曲面を変えずに標準NURBSの制御点を増やす。"""
 nu=len(model.tu)-4;nv=len(model.tv)-4
 if n<max(nu,nv):raise ValueError('細分化で制御点数は減らせません')
 homogeneous=np.column_stack((model.points*model.weights[:,None],model.weights)).reshape(nv,nu,4);tu=model.tu.copy();tv=model.tv.copy()
 while nu<n:
  unique=np.unique(tu);span=np.diff(unique);index=int(np.argmax(span));knot=(unique[index]+unique[index+1])/2;spline=BSpline(tu,homogeneous.transpose(1,0,2).reshape(nu,nv*4),3).insert_knot(knot);tu=spline.t;nu+=1;homogeneous=spline.c[:nu].reshape(nu,nv,4).transpose(1,0,2)
 while nv<n:
  unique=np.unique(tv);span=np.diff(unique);index=int(np.argmax(span));knot=(unique[index]+unique[index+1])/2;spline=BSpline(tv,homogeneous.reshape(nv,nu*4),3).insert_knot(knot);tv=spline.t;nv+=1;homogeneous=spline.c[:nv].reshape(nv,nu,4)
 h=homogeneous.reshape(-1,4);return NurbsSurface(h[:,:3]/h[:,3,None],h[:,3],tu,tv)
