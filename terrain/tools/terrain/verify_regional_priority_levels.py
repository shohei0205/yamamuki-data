"""生成済み地域LODの形状・予算・共有辺を検証する。"""
import json,sqlite3,struct,zlib
import numpy as np
from regional_priority_levels import OUTPUT

def verify():
 with sqlite3.connect(OUTPUT) as db:
  meta=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0]);saved={};summary=[]
  for level,x,y,g,h,m,report in db.execute('SELECT * FROM tiles'):
   r=json.loads(report);raw=zlib.decompress(g);magic,n,nn,nv,ni=struct.unpack_from('<4sHHII',raw)
   assert magic==b'YNT2' and n==nn and len(raw)==16+nv*12+ni*2
   p=np.frombuffer(raw,dtype='<f4',offset=16,count=nv*3).reshape(-1,3);f=np.frombuffer(raw,dtype='<u2',offset=16+nv*12).reshape(-1,3)
   assert np.isfinite(p).all() and f.max()<nv and nv==r['vertices']
   assert r['baseVertices']<=16384 and r['addedVertices']<=1024 and nv==r['baseVertices']+r['addedVertices']
   assert len(r['addedPositions'])==r['addedVertices']
   width=1000*2**level;a=p[f[:,1],:2].astype(float)-p[f[:,0],:2];b=p[f[:,2],:2].astype(float)-p[f[:,0],:2];cross=a[:,0]*b[:,1]-a[:,1]*b[:,0]
   assert np.all(cross>0) and abs(cross.sum()/2-width**2)<width**2*1e-10
   ids=np.sort(np.concatenate([f[:,[0,1]],f[:,[1,2]],f[:,[2,0]]]),axis=1);edge,counts=np.unique(ids,axis=0,return_counts=True);assert counts.max()==2
   for a,b in edge[counts==1]:
    assert any(abs(p[a,k]-v)<.01 and abs(p[b,k]-v)<.01 for k in [0,1] for v in [0,width])
   assert len(zlib.decompress(h))==12+nv*4 and len(zlib.decompress(m))==(n*n+7)//8
   for x0,y0 in p[:,:2]:assert -.01<=x0<=width+.01 and -.01<=y0<=width+.01
   if r.get('boundaryPolicy') in ['adaptive-half-error-v1','quality-synchronized-v1']:
    spacing=width/(n-1);expected=set()
    for name,points in r['edges'].items():
     for at,height in points:
      xy={'north':(at*spacing,width),'south':(at*spacing,0),'west':(0,width-at*spacing),'east':(width,width-at*spacing)}[name]
      expected.add((*xy,height))
    actual={tuple(map(float,v)) for v in p if v[0] in [0,width] or v[1] in [0,width]}
    assert actual==expected and len(actual)==r['boundaryVertices']
   saved[level,x,y]=r
  for (l,x,y),r in saved.items():
   for neighbor,a,b in [((l,x+1,y),'east','west'),((l,x,y+1),'north','south')]:
    if neighbor in saved:np.testing.assert_array_equal(np.array(r['edges'][a]),np.array(saved[neighbor]['edges'][b]))
  for area in meta['reliefRegions']:
   assert [r['level'] for r in area['levels']]==list(range(8))
   for r in area['levels']:
    rows=[saved[tuple(k)] for k in r['keys']];assert sum(q['vertices'] for q in rows)==r['vertices'];summary.append((area['id'],r['level'],r['tileCount'],r['baseVertices'],r['addedVertices'],round(r['bytes']/1024,1),r['baseUnmet']))
  print('検証成功',len(saved),'タイル')
  for row in summary:print(*row)
 return summary

if __name__=='__main__':verify()
