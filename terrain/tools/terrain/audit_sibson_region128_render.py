"""保存曲面と描画三角形の差、および別実装のVoronoi面積座標を確認する。"""
import json,sqlite3,struct
from pathlib import Path
import numpy as np
from scipy.spatial import Voronoi
from shapely.geometry import Polygon
from sibson_region128_server import field,restore,ROOT
from sibson_surface import SibsonSurface
from mls_quant_codec import decode_quant

def reference(model,q):
 old=Voronoi(model.xy);new=Voronoi(np.vstack((model.xy,q)));index=len(model.xy);region=new.regions[new.point_region[index]]
 if -1 in region:raise ValueError('参照照会セルが開いています')
 polygon=Polygon(new.vertices[region]);neighbors=np.unique(new.ridge_points[np.any(new.ridge_points==index,axis=1)]);neighbors=neighbors[neighbors!=index]
 total=0.;weighted=0.
 for i in neighbors:
  region=old.regions[old.point_region[i]]
  if -1 in region:raise ValueError('参照の旧セルが開いています')
  area=polygon.intersection(Polygon(old.vertices[region])).area;total+=area;weighted+=area*model.z[i]
 return weighted/total

def render(tile,q,n):
 size=struct.unpack_from('<I',tile)[0];m=json.loads(tile[4:4+size]);data=np.frombuffer(tile[4+size:],'<f4').reshape(m['ny']+1,m['nx']+1,3)
 uv=(q-m['fieldBounds'][:2])/m['step'];ij=np.floor(uv).astype(int);f=uv-ij;i,j=ij.T;x,y=f.T
 a=data[j,i];b=data[j,i+1];c=data[j+1,i];d=data[j+1,i+1];lower=x+y<=1
 weights=np.column_stack((1-x-y,x,y));weights[~lower]=np.column_stack((x+y-1,1-y,1-x))[~lower]
 values=np.stack((a,b,c),axis=1);values[~lower]=np.stack((d,b,c),axis=1)[~lower]
 h=np.sum(values[:,:,0]*weights,axis=1)
 normal=np.stack((-values[:,:,1],-values[:,:,2],np.ones(values.shape[:2])),axis=2);normal/=np.linalg.norm(normal,axis=2,keepdims=True);normal=np.sum(normal*weights[:,:,None],axis=1);normal/=np.linalg.norm(normal,axis=1,keepdims=True)
 return h,normal

def main():
 region='yari';error=100.;db=sqlite3.connect((ROOT/region/'terrain.sqlite3').resolve().as_uri()+'?mode=ro',uri=True)
 global_model=SibsonSurface(decode_quant(db.execute('SELECT blob FROM models WHERE error=?',(error,)).fetchone()[0]));metadata=json.loads(db.execute('SELECT info FROM models WHERE error=?',(error,)).fetchone()[0]);rows=[];reference_errors=[]
 for tx,ty in [(7,7),(8,7),(7,8),(8,8)]:
  points=restore(db,tx,ty,error);model=SibsonSurface(points);rng=np.random.default_rng(tx*16+ty);q=rng.uniform([tx*8000+250,ty*8000+250],[(tx+1)*8000-250,(ty+1)*8000-250],(4096,2));h=model(q);grad=[]
  for axis in [0,1]:
   shift=np.zeros(2);shift[axis]=1;grad.append((model(q+shift)-model(q-shift))/2)
  normals=np.column_stack((-grad[0],-grad[1],np.ones(len(q))));normals/=np.linalg.norm(normals,axis=1,keepdims=True)
  for z in q[:4]:reference_errors.append(abs(float(model(z[None])[0])-reference(model,z)))
  for n in [32,64,128,256]:
   rendered,normal=render(field(region,tx,ty,error,n),q,n);err=abs(rendered-h);angle=np.arccos(np.clip(np.sum(normal*normals,axis=1),-1,1))*180/np.pi
   rows.append(dict(x=tx,y=ty,n=n,step=8000/n,maxHeightError=float(err.max()),rmse=float(np.sqrt(np.mean(err*err))),maxNormalAngle=float(angle.max()),meanNormalAngle=float(angle.mean()),globalCurveDifference=float(abs(h-global_model(q)).max())))
 db.close();result=dict(region=region,error=error,model=metadata,independentVoronoiQueries=len(reference_errors),independentVoronoiMaxDifference=max(reference_errors),rows=rows)
 (ROOT/'render-audit-yari100.json').write_text(json.dumps(result,indent=2),encoding='utf-8');print(json.dumps(result))

if __name__=='__main__':main()
