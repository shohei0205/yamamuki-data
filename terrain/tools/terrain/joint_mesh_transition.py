"""一括生成 TIN の周囲に旧メッシュと接続する帯を作り、別 SQLite に保存する。"""
import json,sqlite3,struct,zlib
from pathlib import Path
import numpy as np
from scipy.spatial import Delaunay
import national_independent_mesh as national

ROOT=national.ROOT;WIDTH=8000

def decode(blob,x,y):
 raw=zlib.decompress(blob);magic,n,m,nv,ni=struct.unpack_from('<4sHHII',raw)
 if magic==b'YNT2':p=np.frombuffer(raw,'<f4',nv*3,16).reshape(-1,3).astype(float)
 else:
  data=np.array([struct.unpack_from('<HHi',raw,16+i*8) for i in range(nv)],float)
  p=np.column_stack([data[:,0]*WIDTH/(n-1),WIDTH-data[:,1]*WIDTH/(n-1),data[:,2]/4])
 p[:,:2]+=[x*WIDTH,y*WIDTH]
 return p

def build():
 source=ROOT/'build/terrain-independent-national.sqlite3';joint=ROOT/'build/terrain-fuji-joint-l3.sqlite3';output=ROOT/'build/terrain-fuji-joint-connected-l3.sqlite3'
 with sqlite3.connect(source) as old,sqlite3.connect(joint) as new:
  meta=json.loads(new.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0]);core=meta['jointMesh']['bounds'];bounds=[core[0]-WIDTH,core[1]+WIDTH,core[2]-WIDTH,core[3]+WIDTH]
  points={};slopes={};outer={};masks={}
  for x in range(bounds[0]//WIDTH,bounds[1]//WIDTH):
   for y in range(bounds[2]//WIDTH,bounds[3]//WIDTH):
    inside=core[0]<=x*WIDTH<core[1] and core[2]<=y*WIDTH<core[3];db=new if inside else old
    blob,hint,mask=db.execute('SELECT geometry,hints,mask FROM tiles WHERE level=3 AND x=? AND y=?',(x,y)).fetchone();p=decode(blob,x,y);h=zlib.decompress(hint);scale=struct.unpack_from('<f',h,8)[0];g=np.frombuffer(h,'<i2',offset=12).reshape(-1,2)*scale/32767;masks[x,y]=mask
    for v,s in zip(p,g):
     key=tuple(v[:2]);points[key]=v;slopes[key]=s
     if any(v[axis]==edge for axis,edge in [(0,bounds[0]),(0,bounds[1]),(1,bounds[2]),(1,bounds[3])]):outer[key]=v[2]
  p=np.array(list(points.values()));hints=np.array([slopes[tuple(v[:2])] for v in p]);print('接続帯を含む36区画を再三角形分割',len(p),flush=True)
  tin=Delaunay(p[:,:2]);faces=tin.simplices.copy();centers=p[faces,:2].mean(axis=1);owner=np.floor(centers/WIDTH).astype(int)
  # 外周に並ぶ頂点が失われていないことと、旧形状と同じ高さであることを保証する。
  used=np.unique(faces);assert len(used)==len(p)
  point_index={tuple(v[:2]):i for i,v in enumerate(p)}
  for key,z in outer.items():assert p[point_index[key],2]==z
  national.initialize(ROOT/'build/terrain-lod.sqlite3','')
  samples=[];heights=[]
  for x,y in masks:
   xx,yy=np.meshgrid(x*WIDTH+np.linspace(0,WIDTH,257),y*WIDTH+np.linspace(0,WIDTH,257));dem,valid=national.source_samples(xx,yy)
   samples.append(np.column_stack([xx.ravel(),yy.ravel()])[valid.ravel()]);heights.append(np.rint(dem.ravel()[valid.ravel()]*4)/4)
  sample=np.vstack(samples);height=np.concatenate(heights)
  for iteration in range(20):
   tin=Delaunay(p[:,:2]);faces=tin.simplices.copy();ids=tin.find_simplex(sample);assert np.all(ids>=0)
   delta=sample-tin.transform[ids,2];bary=np.einsum('ijk,ik->ij',tin.transform[ids,:2],delta);weights=np.column_stack([bary,1-bary.sum(axis=1)]);pred=(p[faces[ids],2]*weights).sum(axis=1);errors=np.abs(pred-height)
   # 外周の標高列は旧メッシュと同じまま保ち、接続帯の内部だけを改善する。
   interior=(sample[:,0]>bounds[0])&(sample[:,0]<bounds[1])&(sample[:,1]>bounds[2])&(sample[:,1]<bounds[3])
   candidates=np.flatnonzero((errors>25)&interior);order=candidates[np.argsort(-errors[candidates],kind='stable')];_,first=np.unique(ids[order],return_index=True);chosen=order[first]
   chosen=np.array([i for i in chosen if tuple(sample[i]) not in point_index],dtype=int)
   print('接続帯の誤差改善',iteration,float(errors.max()),'追加',len(chosen),flush=True)
   if not len(chosen):break
   assert iteration<19,'誤差の改善が収束しません'
   added=sample[chosen];z=height[chosen];step=7.8125
   xp,_=national.source_samples(added[:,0]+step,added[:,1]);xm,_=national.source_samples(added[:,0]-step,added[:,1]);yp,_=national.source_samples(added[:,0],added[:,1]+step);ym,_=national.source_samples(added[:,0],added[:,1]-step)
   h=np.column_stack([(xp-xm)/(2*step),(ym-yp)/(2*step)])
   for i,v in enumerate(added):point_index[tuple(v)]=len(p)+i
   p=np.vstack([p,np.column_stack([added,z])]);hints=np.vstack([hints,h])
  max_error=float(errors.max());core_valid=(sample[:,0]>=core[0])&(sample[:,0]<=core[1])&(sample[:,1]>=core[2])&(sample[:,1]<=core[3]);core_error=float(errors[core_valid].max())
  centers=p[faces,:2].mean(axis=1);owner=np.floor(centers/WIDTH).astype(int)
  report=dict(region='富士山32km＋8km接続帯',level=3,bounds=bounds,coreBounds=core,maxErrorMeters=max_error,coreMaxErrorMeters=core_error,sampleSpacingMeters=31.25,vertices=len(p),triangles=len(faces),transition=True)
  with sqlite3.connect(output) as db:
   db.executescript('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE IF NOT EXISTS tiles(level INTEGER,x INTEGER,y INTEGER,geometry BLOB,hints BLOB,mask BLOB,stats TEXT,PRIMARY KEY(level,x,y));DELETE FROM tiles;')
   counts=[]
   for (x,y),mask in masks.items():
    chosen=faces[(owner[:,0]==x)&(owner[:,1]==y)];ids,local=np.unique(chosen,return_inverse=True);v=p[ids].copy();v[:,:2]-=[x*WIDTH,y*WIDTH];assert len(ids)<65536
    geometry=zlib.compress(struct.pack('<4sHHII',b'YNT2',1025,1025,len(ids),local.size)+v.astype('<f4').tobytes()+local.astype('<u2').tobytes(),9)
    hint=zlib.compress(struct.pack('<4sIf',b'YNH1',len(ids),8.)+np.rint(np.clip(hints[ids],-8,8)/8*32767).astype('<i2').tobytes(),9)
    db.execute('INSERT INTO tiles VALUES(?,?,?,?,?,?,?)',(3,x,y,geometry,hint,mask,json.dumps(dict(vertices=len(ids),triangles=len(chosen),joint=True))))
    counts.append(len(ids))
   report.update(minVerticesPerTile=min(counts),maxVerticesPerTile=max(counts));meta['jointMesh']=report;db.execute('INSERT OR REPLACE INTO metadata VALUES(?,?)',('manifest',json.dumps(meta)));db.commit()
  print(json.dumps(report,ensure_ascii=False),flush=True)
 return output
if __name__=='__main__':build()
