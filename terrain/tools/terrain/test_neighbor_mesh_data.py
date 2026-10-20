"""隣接面比較の保存形状・追加予算・元頂点の保持を検証する。"""
import json,sqlite3,struct,zlib
from collections import Counter
from pathlib import Path
import numpy as np
ROOT=Path(__file__).resolve().parents[2]

def restore(mode):
 result={}
 with sqlite3.connect(ROOT/f'build/terrain-relief-{mode}-l3.sqlite3') as db:
  meta=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
  for region in meta['reliefRegions']:
   lo,hi,bottom,top=region['bounds'];points=set();edges=Counter();area=0;faces=set();lengths=[];adjacent={}
   for x,y,blob in db.execute('SELECT x,y,geometry FROM tiles WHERE x*8000>=? AND x*8000<? AND y*8000>=? AND y*8000<?',(lo,hi,bottom,top)):
    raw=zlib.decompress(blob);magic,n,m,nv,ni=struct.unpack_from('<4sHHII',raw);assert magic==b'YNT2';p=np.frombuffer(raw,'<f4',nv*3,16).reshape(-1,3).astype(float);p[:,:2]+=[x*8000,y*8000];f=np.frombuffer(raw,'<u2',ni,16+nv*12).reshape(-1,3)
    for ids in f:
     tri=p[ids];keys=[tuple(v) for v in tri];points.update(keys);fk=tuple(sorted(keys));assert fk not in faces;faces.add(fk)
     a=tri[1,:2]-tri[0,:2];b=tri[2,:2]-tri[0,:2];signed=a[0]*b[1]-a[1]*b[0];assert signed>0;area+=signed/2
     at=len(lengths);lengths.append(max(np.linalg.norm(tri[i,:2]-tri[(i+1)%3,:2]) for i in range(3)))
     for i in range(3):
      key=tuple(sorted([keys[i],keys[(i+1)%3]]));edges[key]+=1;adjacent.setdefault(key,[]).append(at)
   assert abs(area-32000**2)<.01;assert max(edges.values())==2
   for (a,b),count in edges.items():
    if count==1:assert any(a[axis]==b[axis]==value for axis,value in [(0,lo),(0,hi),(1,bottom),(1,top)])
   assert len(points)==region['vertices'];assert len(faces)==region['triangles']
   maximum=1.
   for ids in adjacent.values():
    if len(ids)==2:
     a,b=[lengths[i] for i in ids];maximum=max(maximum,a/b,b/a)
   assert abs(maximum-region['balance']['maxRatio'])<1e-8
   result[region['id']]=(points,region)
 return result

if __name__=='__main__':
 base=restore('neighbor0')
 for mode in ['neighbor3','neighbor2']:
  for id,(points,region) in restore(mode).items():
   initial,reference=base[id];assert initial<=points;b=region['balance'];assert b['initialVertices']==len(initial);assert b['addedVertices']==len(points)-len(initial);assert b['addedVertices']<=len(initial)*15//100
   print(mode,id,'追加',b['addedVertices'],'/',len(initial),'最大比',round(b['maxRatio'],3),'標高条件',region['toleranceReached'])
