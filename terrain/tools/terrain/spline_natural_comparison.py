"""Bスプラインと同じ入力点からSibson曲面を評価する比較パケット。"""
import base64,json,struct,urllib.request,gzip
import numpy as np
from sibson_surface import SibsonSurface

def packet(region):
 with urllib.request.urlopen('http://127.0.0.1:61473/api/ridge-spline-input?region='+region,timeout=120) as response:
  body=response.read();source=json.loads(gzip.decompress(body) if body[:2]==b'\x1f\x8b' else body)
 p=np.frombuffer(base64.b64decode(source['points']),'<f4').reshape(-1,3).astype(float);model=SibsonSurface(p)
 n=512;axisx=np.linspace(model.lower[0],model.upper[0],n+1);axisy=np.linspace(model.lower[1],model.upper[1],n+1);x,y=np.meshgrid(axisx,axisy);q=np.column_stack((x.ravel(),y.ravel()));height=model(q);grad=[]
 for c in [0,1]:
  a=q.copy();b=q.copy();a[:,c]=np.minimum(model.upper[c],a[:,c]+1);b[:,c]=np.maximum(model.lower[c],b[:,c]-1);grad.append((model(a)-model(b))/(a[:,c]-b[:,c]))
 normal=np.column_stack((-grad[0],np.ones(len(q)),-grad[1]));normal/=np.linalg.norm(normal,axis=1)[:,None];v=np.column_stack((q[:,0],height,q[:,1],normal)).astype('<f4');a=(np.arange(n)[:,None]*(n+1)+np.arange(n)[None,:]).ravel();faces=np.concatenate((np.column_stack((a,a+n+2,a+1)),np.column_stack((a,a+n+1,a+n+2)))).astype('<u4');info=dict(region=region,points=len(p),vertices=len(v),triangles=len(faces),method='Sibson',grid=n,source='B-spline approximation input')
 linear=p[model.tri].copy();linear=linear[:,:,[0,2,1]];normals=np.cross(linear[:,1]-linear[:,0],linear[:,2]-linear[:,0]);normals[normals[:,1]<0]*=-1;normals/=np.maximum(np.linalg.norm(normals,axis=1)[:,None],1e-12);lv=np.concatenate((linear,np.repeat(normals[:,None,:],3,axis=1)),axis=2).reshape(-1,6).astype('<f4');info['linearVertices']=len(lv)
 header=json.dumps(info).encode();return struct.pack('<III',len(header),len(v),len(faces))+header+v.tobytes()+faces.tobytes()+lv.tobytes()
