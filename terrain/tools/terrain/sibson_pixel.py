"""画素単位Sibson評価に必要な制御点・旧Voronoi半平面・候補表を作る。"""
import json,struct
import numpy as np
from sibson_surface import SibsonSurface

MAX_CANDIDATES=48
MAX_PLANES=64

def build(points,bounds):
 model=SibsonSurface(points);bounds=np.asarray(bounds,float);origin=(bounds[:2]+bounds[2:])*.5
 core=np.all((model.xy>=bounds[:2])&(model.xy<=bounds[2:]),axis=1)
 n=int(np.clip(2**np.ceil(np.log2(max(16,np.sqrt(int(core.sum()))*2))),16,256));step=(bounds[2]-bounds[0])/n
 offsets=np.zeros(len(points),np.uint32);records=[]
 for i,p in enumerate(points):
  offsets[i]=len(records);neighbors=model.links[model.off[i]:model.off[i+1]];delta=model.xy[neighbors]-model.xy[i];distance=np.linalg.norm(delta,axis=1)
  records.append([p[0]-origin[0],p[1]-origin[1],p[2],len(neighbors)])
  for d,length in zip(delta,distance):records.append([d[0]/length,d[1]/length,length*.5,0])
 cells=[set() for _ in range(n*n)]
 for face,(cx,cy,r2) in zip(model.tri,model.circles):
  radius=np.sqrt(max(0,r2));lo=np.floor((np.array([cx-radius,cy-radius])-bounds[:2])/step).astype(int);hi=np.floor((np.array([cx+radius,cy+radius])-bounds[:2])/step).astype(int)
  lo=np.maximum(lo,0);hi=np.minimum(hi,n-1)
  if np.any(lo>hi):continue
  xs=np.arange(lo[0],hi[0]+1);nearest_x=np.clip(cx,bounds[0]+xs*step,bounds[0]+(xs+1)*step)
  for y in range(lo[1],hi[1]+1):
   nearest_y=np.clip(cy,bounds[1]+y*step,bounds[1]+(y+1)*step)
   selected=xs[(nearest_x-cx)**2+(nearest_y-cy)**2<=r2*(1+1e-12)+1e-5]
   for x in selected:cells[y*n+x].update(int(v) for v in face)
 meta=np.zeros((n*n,2),'<u4');refs=[];unsupported=0;maximum=0
 for i,cell in enumerate(cells):
  ids=sorted(cell);maximum=max(maximum,len(ids));meta[i,0]=len(refs)
  if not ids or len(ids)>MAX_CANDIDATES or any(model.off[v+1]-model.off[v]>MAX_PLANES for v in ids):meta[i,1]=0xffffffff;unsupported+=1
  else:meta[i,1]=len(ids);refs.extend(int(offsets[v]) for v in ids)
 data=np.asarray(records,'<f4');ref=np.asarray(refs,'<u4')
 info=dict(origin=origin.tolist(),bounds=bounds.tolist(),gridN=n,step=step,pointRecords=len(data),referenceCount=len(ref),controlPoints=len(points),unsupportedCells=unsupported,totalCells=n*n,maxCandidates=maximum,limits=dict(candidates=MAX_CANDIDATES,planes=MAX_PLANES,polygonVertices=24),pointBytes=data.nbytes,cellBytes=meta.nbytes,referenceBytes=ref.nbytes)
 header=json.dumps(info).encode();return struct.pack('<I',len(header))+header+data.tobytes()+meta.tobytes()+ref.tobytes()
