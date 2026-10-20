"""品質補修後の頂点保持・Delaunay条件・鋭角面・標高誤差を従来版と比較する。"""
import argparse,json,sqlite3,struct,zlib
from pathlib import Path
import numpy as np
from refine_regional_quality import angles

def decode(blob):
 raw=zlib.decompress(blob);_,n,_,nv,ni=struct.unpack_from('<4sHHII',raw);p=np.frombuffer(raw,dtype='<f4',offset=16,count=nv*3).reshape(-1,3).astype(float);f=np.frombuffer(raw,dtype='<u2',offset=16+nv*12).reshape(-1,3);return p,f

def delaunay_check(p,f):
 edges=np.concatenate([f[:,[0,1,2]],f[:,[1,2,0]],f[:,[2,0,1]]]);keys=np.sort(edges[:,:2],axis=1);order=np.lexsort((keys[:,1],keys[:,0]));same=np.flatnonzero(np.all(keys[order][1:]==keys[order][:-1],axis=1));left=edges[order[same]];right=edges[order[same+1]];a=p[left[:,0],:2]-p[right[:,2],:2];b=p[left[:,1],:2]-p[right[:,2],:2];c=p[left[:,2],:2]-p[right[:,2],:2]
 aa=(a*a).sum(axis=1);bb=(b*b).sum(axis=1);cc=(c*c).sum(axis=1);det=aa*(b[:,0]*c[:,1]-b[:,1]*c[:,0])-bb*(a[:,0]*c[:,1]-a[:,1]*c[:,0])+cc*(a[:,0]*b[:,1]-a[:,1]*b[:,0]);u=b-a;v=c-a;orientation=u[:,0]*v[:,1]-u[:,1]*v[:,0];bound=np.maximum(np.maximum(aa,bb),cc)**2*1e-9
 assert np.all(det*np.sign(orientation)<=bound+1e-7),'Delaunay条件の違反'

def verify(source,output):
 reports=[];boundaries={}
 with sqlite3.connect(source) as old,sqlite3.connect(output) as new:
  for level,x,y,g,h,report in new.execute('SELECT level,x,y,geometry,hints,stats FROM tiles'):
   previous=old.execute('SELECT geometry,hints,stats FROM tiles WHERE level=? AND x=? AND y=?',(level,x,y)).fetchone();p,f=decode(g);before,bf=decode(previous[0]);r=json.loads(report);o=json.loads(previous[2]);assert np.array_equal(p[:len(before)],before),'元頂点の変更';assert set(f.ravel())==set(range(len(p))),'未使用頂点';delaunay_check(p,f)
   assert zlib.decompress(h)[12:12+len(before)*4]==zlib.decompress(previous[1])[12:],'元の傾きヒントの変更'
   assert r['maxErrorMeters']<=max([2,5,10,25,50,100,200,400][level],o['maxErrorMeters'])+1e-5
   encoded=np.frombuffer(zlib.decompress(h),dtype='<i2',offset=12).reshape(-1,2);width=1000*2**level
   boundaries[level,x,y]={name:{float(p[i,coordinate]):(float(p[i,2]),tuple(encoded[i])) for i in np.flatnonzero(p[:,axis]==value)} for name,axis,value,coordinate in [('north',1,width,0),('south',1,0,0),('west',0,0,1),('east',0,width,1)]}
   a,b=angles(before,bf),angles(p,f);reports.append(dict(level=level,x=x,y=y,beforeUnder5=int((a<5).sum()),afterUnder5=int((b<5).sum()),beforeUnder10=int((a<10).sum()),afterUnder10=int((b<10).sum()),verticesBefore=len(before),verticesAfter=len(p)))
 for (l,x,y),edges in boundaries.items():
  for neighbor,a,b in [((l,x+1,y),'east','west'),((l,x,y+1),'north','south')]:
   if neighbor in boundaries:assert edges[a]==boundaries[neighbor][b],'共有辺の高さ・傾きが不一致'
 totals=[]
 for level in range(8):
  rows=[r for r in reports if r['level']==level];entry=dict(level=level,**{k:sum(r[k] for r in rows) for k in ['beforeUnder5','afterUnder5','beforeUnder10','afterUnder10','verticesBefore','verticesAfter']});totals.append(entry);print(entry,flush=True)
 assert len(reports)==144
 print('144タイル：元頂点・傾き保持、Delaunay条件、標高誤差を確認')
 return dict(tiles=reports,levels=totals)
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('source',type=Path);p.add_argument('output',type=Path);p.add_argument('--report',type=Path);a=p.parse_args();r=verify(a.source,a.output)
 if a.report:a.report.write_text(json.dumps(r,ensure_ascii=False,indent=2),encoding='utf-8')
