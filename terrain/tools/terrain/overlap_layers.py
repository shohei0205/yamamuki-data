"""タイルの領域に交差する面と、外側の隣接面を層数で切り出す。"""
from functools import lru_cache
import json
import numpy as np

@lru_cache(maxsize=8)
def index(mesh_key,raw):
 mesh=json.loads(raw);points=np.asarray(mesh['points']);faces=np.asarray(mesh['faces'],dtype=np.int64);tri=points[faces,:2]
 neighbors=[set() for _ in faces];edges={}
 for fi,f in enumerate(faces):
  for k in range(3):
   edge=tuple(sorted((int(f[k]),int(f[(k+1)%3]))));other=edges.setdefault(edge,[])
   assert len(other)<2,'面が3枚以上接続する辺'
   if other:neighbors[fi].add(other[0]);neighbors[other[0]].add(fi)
   other.append(fi)
 return mesh,tri,neighbors

def extract(mesh,rect,layers=2):
 assert layers in range(5)
 raw=json.dumps(dict(points=mesh['points'],faces=mesh['faces']),separators=(',',':'))
 source,tri,neighbors=index(mesh['meshKey'],raw)
 lo=np.asarray(rect[:2]);hi=np.asarray(rect[2:]);center=(lo+hi)/2;half=(hi-lo)/2
 intersects=np.all(tri.max(axis=1)>=lo-1e-7,axis=1)&np.all(tri.min(axis=1)<=hi+1e-7,axis=1)
 # 分離軸判定で、包囲矩形だけが重なる三角形を除く。
 for k in range(3):
  edge=tri[:,(k+1)%3]-tri[:,k];normal=np.column_stack((-edge[:,1],edge[:,0]));projection=np.sum((tri-center)*normal[:,None,:],axis=2);radius=np.sum(np.abs(normal)*half,axis=1)
  intersects&=(projection.min(axis=1)<=radius+1e-6)&(projection.max(axis=1)>=-radius-1e-6)
 selected={int(i):0 for i in np.flatnonzero(intersects)};front=set(selected)
 for layer in range(1,layers+1):
  front={j for i in front for j in neighbors[i] if j not in selected}
  selected.update((i,layer) for i in front)
 face_ids=sorted(selected);fs=[source['faces'][i] for i in face_ids];ids=sorted({i for f in fs for i in f});remap={v:i for i,v in enumerate(ids)}
 return dict(meshKey=mesh['meshKey'],bounds=rect,points=[source['points'][i] for i in ids],faces=[[remap[i] for i in f] for f in fs],support=[],haloLayers=layers,bodyFaces=sum(v==0 for v in selected.values()),haloFaces=sum(v>0 for v in selected.values()))

def reach(tile):
 x,y,u,v=tile['bounds']
 return max((max(x-p[0],y-p[1],p[0]-u,p[1]-v,0) for p in tile['points']),default=0)
