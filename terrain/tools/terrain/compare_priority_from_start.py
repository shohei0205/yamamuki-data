"""同じ境界・頂点数で二段階TINと最初から比高優先のTINを比較する。"""
import argparse,json,math,os,sqlite3,struct,subprocess,tempfile,zlib,time
from concurrent.futures import ProcessPoolExecutor,as_completed
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.ndimage import map_coordinates
import national_independent_mesh as national
import refine_regional_quality as quality
from tin_vertex_optimize import raster
ROOT=national.ROOT

def initialize():national.initialize(ROOT/'build/terrain-lod.sqlite3',str(ROOT/'build/terrain-rust-target/debug/terrain-adaptive-tin.exe'))

def measure(p,f,grid,limits,valid):
 owner,w=raster(p,f,len(grid));pred=(p[f[owner],2]*w).sum(axis=1).reshape(grid.shape);e=np.abs(pred-grid);v=e[valid];return dict(maximum=float(v.max()),rmse=float(np.sqrt((v*v).mean())),weightedMaximum=float((e/limits)[valid].max()),mean=float(v.mean()))

def generate_tile(row,budget=None):
 l,x,y,g,h,m,meta=row;raw=zlib.decompress(g);_,n,_,nv,ni=struct.unpack_from('<4sHHII',raw);width=1000*2**l;spacing=width/(n-1);old=np.frombuffer(raw,dtype='<f4',count=nv*3,offset=16).reshape(-1,3).astype(float);old[:,:2]/=spacing;oldf=np.frombuffer(raw,dtype='<u2',count=ni,offset=16+nv*12).reshape(-1,3)
 grid,limits,gx,gy=quality.source_fields(l,x,y,n);valid=np.unpackbits(np.frombuffer(zlib.decompress(m),dtype=np.uint8),bitorder='little')[:n*n].reshape(n,n).astype(bool)
 seeds=old[np.any((old[:,:2]==0)|(old[:,:2]==n-1),axis=1),:2].astype(int).tolist();results=[];original=measure(old,oldf,grid,limits,valid)
 if budget is not None:nv=budget
 if len(seeds)>nv:raise ValueError('固定境界点が頂点上限を超えます')
 with tempfile.TemporaryDirectory(dir=ROOT/'build',prefix='unified-tin-') as folder:
  folder=Path(folder);offset=min(0,float(grid.min()));Image.fromarray(np.rint((grid-offset)*4).astype('uint16')).save(folder/'height.png');Image.fromarray(np.rint(limits*100).astype('uint16')).save(folder/'limits.png')
  def run(seed,name,weighted,count,minimum):
   (folder/'seed.json').write_text(json.dumps(seed),encoding='utf-8');cmd=[national.BINARY,'--input',str(folder/'height.png'),'--output',str(folder/(name+'.json')),'--scale','.25','--offset',str(offset),'--max-error',str(quality.ERRORS[l]),'--min-vertices',str(minimum),'--max-vertices',str(count),'--seed-points',str(folder/'seed.json'),'--lock-seed-boundary']
   if weighted:cmd+=['--error-limits',str(folder/'limits.png')]
   subprocess.run(cmd,check=True,capture_output=True,text=True);return json.loads((folder/(name+'.json')).read_text(encoding='utf-8'))
  for mode in (['unified'] if budget is not None else ['staged','unified']):
   start=time.time();seed=seeds
   if mode=='staged':base=run(seed,'base',False,nv,4);seed=np.asarray(base['vertices'])[:,:2].astype(int).tolist()
   mesh=run(seed,mode,True,nv,nv);p=np.asarray(mesh['vertices'],dtype=float);f=np.asarray(mesh['triangles'],dtype=np.uint32);a=p[f[:,1],:2]-p[f[:,0],:2];b=p[f[:,2],:2]-p[f[:,0],:2];negative=a[:,0]*b[:,1]-a[:,1]*b[:,0]<0;f[negative]=f[negative][:,[0,2,1]]
   if len(p)!=nv:raise ValueError(f'頂点数が一致しません {(l,x,y,mode)}: {len(p)} != {nv}')
   # 境界点の集合を変えず、両方式に同じ条件を与える。
   boundary=p[np.any((p[:,:2]==0)|(p[:,:2]==n-1),axis=1),:2].astype(int)
   if {tuple(v) for v in boundary}!={tuple(v) for v in seeds}:raise ValueError('境界点が変化しました')
   metrics=measure(p,f,grid,limits,valid);world=p.copy();world[:,:2]*=spacing;geom=zlib.compress(struct.pack('<4sHHII',b'YNT2',n,n,nv,f.size)+world.astype('<f4').tobytes()+f.astype('<u2').tobytes(),9)
   slopes=np.column_stack([map_coordinates(gx,[p[:,1],p[:,0]],order=1),-map_coordinates(gy,[p[:,1],p[:,0]],order=1)]);hint=zlib.compress(struct.pack('<4sIf',b'YNH1',nv,8.)+np.rint(np.clip(slopes,-8,8)/8*32767).astype('<i2').tobytes(),9)
   stats=dict(mode=mode,vertices=nv,boundaryVertices=len(seeds),metrics=metrics,originalMetrics=original,geometryBytes=len(geom),sampleGridSize=n,seconds=time.time()-start,minimumAngle=float(quality.angles(p,f).min()))
   results.append((mode,(l,x,y,geom,hint,m,json.dumps(stats)),stats))
 return results

def main(args):
 source=ROOT/'build/terrain-regional-priority-levels-v5.sqlite3';src=sqlite3.connect(source.resolve().as_uri()+'?mode=ro',uri=True);manifest=json.loads(src.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0]);rows=src.execute('SELECT * FROM tiles WHERE level IN ('+','.join('?'*len(args.levels))+') ORDER BY level,x,y',args.levels).fetchall();dbs={}
 for mode in ['staged','unified']:
  path=ROOT/f'build/terrain-priority-{mode}-compare.sqlite3';db=sqlite3.connect(path);db.executescript('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE IF NOT EXISTS tiles(level,x,y,geometry,hints,mask,stats,PRIMARY KEY(level,x,y));');dbs[mode]=db
 complete={tuple(r) for r in dbs['staged'].execute('SELECT level,x,y FROM tiles')}&{tuple(r) for r in dbs['unified'].execute('SELECT level,x,y FROM tiles')};pending=[r for r in rows if tuple(r[:3]) not in complete]
 print('全',len(rows),'残り',len(pending),flush=True)
 with ProcessPoolExecutor(max_workers=args.workers,initializer=initialize) as pool:
  futures=[pool.submit(generate_tile,row) for row in pending]
  for i,future in enumerate(as_completed(futures)):
   for mode,row,stats in future.result():dbs[mode].execute('INSERT OR REPLACE INTO tiles VALUES(?,?,?,?,?,?,?)',row);dbs[mode].commit()
   (ROOT/'build/terrain-priority-comparison.progress.json').write_text(json.dumps(dict(done=len(complete)+i+1,total=len(rows),key=row[:3])),encoding='utf-8');print(len(complete)+i+1,'/',len(rows),row[:3],flush=True)
 reports={}
 for mode,db in dbs.items():
  m=json.loads(json.dumps(manifest));m['levels']=[v for v in m['levels'] if v['level'] in args.levels];m.update(priorityComparison=True,priorityMode=mode,priorityComparisonPolicy='same-boundary-and-exact-vertex-count-v1')
  for area in m['reliefRegions']:
   area['levels']=[v for v in area['levels'] if v['level'] in args.levels]
   for spec in area['levels']:
    stats=[json.loads(db.execute('SELECT stats FROM tiles WHERE level=? AND x=? AND y=?',key).fetchone()[0]) for key in spec['keys']];spec['addedPositions']=[];spec['priorityMetrics']=dict(maximum=max(r['metrics']['maximum'] for r in stats),rmse=math.sqrt(np.mean([r['metrics']['rmse']**2 for r in stats])),weightedMaximum=max(r['metrics']['weightedMaximum'] for r in stats),vertices=sum(r['vertices'] for r in stats),geometryBytes=sum(r['geometryBytes'] for r in stats));reports[mode,area['id'],spec['level']]=spec['priorityMetrics']
  db.execute('INSERT OR REPLACE INTO metadata VALUES("manifest",?)',(json.dumps(m),));db.commit()
 report=[dict(mode=k[0],region=k[1],level=k[2],**v) for k,v in reports.items()];(ROOT/'build/terrain-priority-comparison-report.json').write_text(json.dumps(report,indent=2),encoding='utf-8');(ROOT/'build/terrain-priority-comparison.ready').write_text('ready',encoding='utf-8');print(json.dumps(report),flush=True)

if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--levels',nargs='+',type=int,default=[3,4]);p.add_argument('--workers',type=int,default=2);main(p.parse_args())
