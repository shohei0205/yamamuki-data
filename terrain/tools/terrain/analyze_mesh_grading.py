"""ピーク周辺2.5 kmの隣接面の大きさの差を比較する。"""
import sqlite3,struct,zlib,json
import numpy as np
from national_independent_mesh import project
from pathlib import Path
for mode in ['peaks','graded','neighbor0','neighbor3','neighbor2']:
 with sqlite3.connect(Path(__file__).resolve().parents[2]/'build'/f'terrain-relief-{mode}-l3.sqlite3') as db:
  for name,lon,lat in [('fuji',138.7274,35.3606),('tsukuba',140.1069,36.2254),('yari',137.6476,36.3419)]:
   cx,cy=project(lon,lat);edges={};lengths=[];centers=[]
   for x,y,blob in db.execute('SELECT x,y,geometry FROM tiles WHERE level=3'):
    if abs((x+.5)*8000-cx)>24000 or abs((y+.5)*8000-cy)>24000:continue
    raw=zlib.decompress(blob);_,_,_,nv,ni=struct.unpack_from('<4sHHII',raw);p=np.frombuffer(raw,'<f4',nv*3,16).reshape(-1,3).astype(float);p[:,:2]+=[x*8000,y*8000];f=np.frombuffer(raw,'<u2',ni,16+nv*12).reshape(-1,3)
    for face in f:
     tri=p[face,:2];at=len(lengths);lengths.append(max(np.linalg.norm(tri[i]-tri[(i+1)%3]) for i in range(3)));centers.append(tri.mean(axis=0))
     for i in range(3):
      a=tuple(tri[i]);b=tuple(tri[(i+1)%3]);key=tuple(sorted([a,b]));edges.setdefault(key,[]).append(at)
   ratios=[]
   for (a,b),ids in edges.items():
    if len(ids)!=2 or np.linalg.norm((np.array(a)+b)/2-[cx,cy])>2500:continue
    left,right=[lengths[i] for i in ids];ratios.append(max(left/right,right/left))
   print(mode,name,'edges',len(ratios),'p95',np.quantile(ratios,.95),'ratio>2',np.mean(np.array(ratios)>2),'max',max(ratios))
