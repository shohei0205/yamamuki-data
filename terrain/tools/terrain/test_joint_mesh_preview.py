"""一括 TIN の保存後に、面の重複・穴・反転と境界横断を検証する。"""
import json,sqlite3,struct,zlib
from collections import Counter
from pathlib import Path
import numpy as np

path=Path(__file__).resolve().parents[2]/'build/terrain-fuji-joint-l3.sqlite3'
with sqlite3.connect(path) as db:
 meta=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])['jointMesh']
 bounds=meta['bounds'];edges=Counter();face_keys=set();area=0;crossing=0;sizes=[]
 for x,y,blob,hints,stats in db.execute('SELECT x,y,geometry,hints,stats FROM tiles'):
  raw=zlib.decompress(blob);magic,n,m,nv,ni=struct.unpack_from('<4sHHII',raw);assert magic==b'YNT2'
  p=np.frombuffer(raw,'<f4',nv*3,16).reshape(-1,3).astype(float);p[:,:2]+=[x*8000,y*8000]
  f=np.frombuffer(raw,'<u2',ni,16+nv*12).reshape(-1,3);assert f.max()<nv
  centers=p[f,:2].mean(axis=1);assert np.all(np.floor(centers/8000)==[x,y])
  a=p[f[:,1],:2]-p[f[:,0],:2];b=p[f[:,2],:2]-p[f[:,0],:2];signed=a[:,0]*b[:,1]-a[:,1]*b[:,0];assert np.all(signed>0);area+=signed.sum()/2
  keys=[tuple(v) for v in p];crossing+=sum(np.any(np.floor(p[t,:2]/8000)!=[x,y]) for t in f)
  for t in f:
   k=tuple(sorted(keys[i] for i in t));assert k not in face_keys;face_keys.add(k)
   for i in range(3):edges[tuple(sorted([keys[t[i]],keys[t[(i+1)%3]]]))]+=1
  sizes.append((nv,len(f),len(blob),len(hints)))
 assert max(edges.values())==2
 for (a,b),count in edges.items():
  if count==1:assert any(a[axis]==b[axis]==bound for axis,bound in [(0,bounds[0]),(0,bounds[1]),(1,bounds[2]),(1,bounds[3])]),(a,b)
 assert abs(area-32000**2)<.01
 assert meta['maxErrorMeters']<=25
 assert crossing>0
 print(json.dumps(dict(tiles=len(sizes),faces=len(face_keys),crossingTriangles=int(crossing),areaKm2=area/1e6,minVertices=min(s[0] for s in sizes),maxVertices=max(s[0] for s in sizes),geometryBytes=sum(s[2] for s in sizes),hintBytes=sum(s[3] for s in sizes),maxError=meta['maxErrorMeters'])))
