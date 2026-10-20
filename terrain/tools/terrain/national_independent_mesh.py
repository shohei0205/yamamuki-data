"""L0～L7の独立TINと曲面の傾きヒントを全国生成・再開する。"""
import argparse,hashlib,json,math,os,shutil,sqlite3,struct,subprocess,tempfile,time,zlib
from collections import OrderedDict
from concurrent.futures import ProcessPoolExecutor,wait,FIRST_COMPLETED
from pathlib import Path
import numpy as np
from PIL import Image
from scipy import sparse
from scipy.ndimage import map_coordinates
from scipy.sparse.linalg import lsmr
from scipy.spatial import Delaunay
from compact_dem import decode
from curve_hints import design
from tin_vertex_optimize import raster
from national_mesh_levels import definitions
ROOT=Path(__file__).resolve().parents[2]
R=6378137.;P1=math.radians(30);P2=math.radians(46);LON0=math.radians(138);LAT0=math.radians(36)
NN=math.log(math.cos(P1)/math.cos(P2))/math.log(math.tan(math.pi/4+P2/2)/math.tan(math.pi/4+P1/2))
FF=math.cos(P1)*math.tan(math.pi/4+P1/2)**NN/NN
RHO0=R*FF/math.tan(math.pi/4+LAT0/2)**NN

def project(lon,lat):
 lat=np.radians(lat);theta=NN*(np.radians(lon)-LON0);rho=R*FF/np.tan(np.pi/4+lat/2)**NN
 return rho*np.sin(theta),RHO0-rho*np.cos(theta)

def inverse(x,y):
 rho=np.hypot(x,RHO0-y);theta=np.arctan2(x,RHO0-y)
 return np.degrees(theta/NN+LON0),np.degrees(2*np.arctan((R*FF/rho)**(1/NN))-np.pi/2)

def open_source(path):return sqlite3.connect(Path(path).resolve().as_uri()+'?mode=ro',uri=True)

def initialize(source,binary):
 global SRC,BINARY,CACHE
 SRC=open_source(source);BINARY=binary;CACHE=OrderedDict()
 os.environ['OPENBLAS_NUM_THREADS']='1';os.environ['OMP_NUM_THREADS']='1'

def source_samples(x,y):
 lon,lat=inverse(x,y);px=(lon+180)/360*(16384*256)-.5;py=(1-np.arcsinh(np.tan(np.radians(lat)))/np.pi)/2*(16384*256)-.5
 ix=np.floor(px).astype(np.int64);iy=np.floor(py).astype(np.int64);fx=px-ix;fy=py-iy
 result=np.zeros(px.shape);weight=np.zeros(px.shape)
 for dx,dy,w in [(0,0,(1-fx)*(1-fy)),(1,0,fx*(1-fy)),(0,1,(1-fx)*fy),(1,1,fx*fy)]:
  gx=(ix+dx).ravel();gy=(iy+dy).ravel();keys=(gx//256)*16384+gy//256
  order=np.argsort(keys);cuts=np.r_[0,np.flatnonzero(np.diff(keys[order]))+1,len(order)]
  for a,b in zip(cuts[:-1],cuts[1:]):
   ids=order[a:b];key=int(keys[ids[0]])
   if key not in CACHE:
    row=SRC.execute('SELECT data FROM tiles WHERE z=14 AND x=? AND y=?',(key//16384,key%16384)).fetchone()
    CACHE[key]=decode(row[0]) if row else None
    if len(CACHE)>128:CACHE.popitem(last=False)
   else:CACHE.move_to_end(key)
   tile=CACHE[key]
   if tile is None:continue
   values=tile[gy[ids]%256,gx[ids]%256];valid=np.isfinite(values);ids=ids[valid];values=values[valid];ww=w.ravel()[ids]
   result.ravel()[ids]+=values*ww;weight.ravel()[ids]+=ww
 valid=weight>.999999
 return np.where(valid,result,0),valid

def encode_mesh(points,faces,n):
 raw=struct.pack('<4sHHII',b'YNT1',n,n,len(points),faces.size)
 q=points.copy();q[:,2]=np.rint(q[:,2]*4)
 for x,y,z in q:raw+=struct.pack('<HHi',int(x),int(y),int(z))
 raw+=faces.astype('<u2').tobytes()
 return zlib.compress(raw,9)

def fit_hints(points,faces,grid,halo,width,valid_mask):
 n=len(grid);stride=max(1,(n-1)//128);small=grid[::stride,::stride];owner,w=raster(np.column_stack([points[:,:2]/stride,points[:,2]]),faces,len(small))
 world=points.copy();world[:,:2]*=width/(n-1);base,A=design(world,faces,owner,w)
 spacing=width/(n-1);dy,dx=np.gradient(halo,spacing);coords=points[:,:2].T+1
 initial=np.clip(np.column_stack([map_coordinates(dx,[coords[1],coords[0]],order=1),map_coordinates(dy,[coords[1],coords[0]],order=1)]).ravel(),-8,8)
 boundary=np.any((points[:,:2]==0)|(points[:,:2]==n-1),axis=1);free=np.repeat(~boundary,2)
 # 共有辺の曲線を一致させるため、境界の傾きは同じ元標高から算出して固定する。
 fit_valid=valid_mask[::stride,::stride].ravel()
 if not fit_valid.any():fit_valid[:]=True
 delta=lsmr(A[fit_valid][:,free],small.ravel()[fit_valid]-base[fit_valid]-A[fit_valid]@initial,damp=5.,atol=1e-5,btol=1e-5,maxiter=80)[0]
 slopes=initial.copy();slopes[free]+=delta;encoded=np.rint(np.clip(slopes,-8,8)/8*32767).astype('<i2')
 body=zlib.compress(struct.pack('<4sIf',b'YNH1',len(points),8.)+encoded.tobytes(),9)
 predicted=base+A@(encoded.astype(float)*8/32767)
 return body,float(np.sqrt(np.mean((predicted[fit_valid]-small.ravel()[fit_valid])**2)))

def generate_tile(job):
 level,tx,ty=job;spec=definitions()[level];width=spec['widthMeters'];n=min(1025,129*2**level-(2**level-1));spacing=width/(n-1)
 coords=np.arange(-1,n+1)*spacing;xx,yy=np.meshgrid(tx*width+coords,(ty+1)*width-coords)
 halo,valid=source_samples(xx,yy);grid=halo[1:-1,1:-1];mask=valid[1:-1,1:-1]
 if not mask.any():return job,None
 grid=np.rint(grid*4)/4;halo=np.rint(halo*4)/4
 offset=min(0,float(grid.min()));pixels=np.rint((grid-offset)*4)
 if pixels.max()>65535:raise ValueError('標高の量子化範囲を超過')
 with tempfile.TemporaryDirectory(prefix='national-tin-',dir=ROOT/'build') as tmp:
  png=Path(tmp)/'height.png';report=Path(tmp)/'mesh.json';Image.fromarray(pixels.astype(np.uint16)).save(png)
  run=subprocess.run([BINARY,'--input',str(png),'--output',str(report),'--scale','.25','--offset',str(offset),'--max-error',str(spec['maxErrorMeters']),'--min-vertices','128','--max-vertices','1024','--seamless'],capture_output=True,text=True,encoding='utf-8')
  if run.returncode:raise RuntimeError(run.stderr.strip())
  mesh=json.loads(report.read_text(encoding='utf-8'));points=np.asarray(mesh['vertices']);faces=np.asarray(mesh['triangles'],np.uint16)
 if len(points)<128:
  # 完全な平面で誤差優先挿入が停止する場合も、指定された最低頂点数を満たす。
  axis=np.linspace(1,n-2,12).round().astype(int);candidates=np.array([(x,y) for y in axis for x in axis]);ids=np.linspace(0,len(candidates)-1,128-len(points)).round().astype(int);xy=candidates[ids]
  points=np.vstack([points,np.column_stack([xy,grid[xy[:,1],xy[:,0]]])]);faces=Delaunay(points[:,:2]).simplices.astype(np.uint16)
  tri=points[faces];area=np.cross(tri[:,1,:2]-tri[:,0,:2],tri[:,2,:2]-tri[:,0,:2]);faces[area<0]=faces[area<0][:,[0,2,1]]
 if not 128<=len(points)<=1024:raise ValueError('頂点数制約違反')
 owner,w=raster(points,faces,n);predicted=(points[faces[owner],2]*w).sum(axis=1);error=np.abs(predicted-grid.ravel());valid_error=error[mask.ravel()]
 geometry=encode_mesh(points,faces,n);hints,curve_rmse=fit_hints(points,faces,grid,halo,width,mask);mask_blob=zlib.compress(np.packbits(mask,bitorder='little').tobytes(),9)
 slopes=np.frombuffer(zlib.decompress(hints),'<i2',offset=12).reshape(-1,2);edges=[]
 for axis,at,along in [(0,0,1),(0,n-1,1),(1,0,0),(1,n-1,0)]:
  ids=np.flatnonzero(points[:,axis]==at);ids=ids[np.argsort(points[ids,along])];packet=b''.join(struct.pack('<Hihh',int(points[i,along]),round(points[i,2]*4),*slopes[i]) for i in ids);edges.append(hashlib.sha256(packet).hexdigest())
 stats=dict(boundaryDigests=edges,vertices=len(points),triangles=len(faces),sampleGridSize=n,sampleSpacingMeters=spacing,validSamples=int(mask.sum()),maxErrorMeters=float(valid_error.max()),rmseMeters=float(np.sqrt(np.mean(valid_error**2))),curveFitSampleRmseMeters=curve_rmse,toleranceReached=bool(valid_error.max()<=spec['maxErrorMeters']),geometryBytes=len(geometry),hintBytes=len(hints),maskBytes=len(mask_blob))
 return job,(geometry,hints,mask_blob,json.dumps(stats))

def catalog(db,source):
 if db.execute('SELECT count(*) FROM jobs').fetchone()[0]:return
 with open_source(source) as src:tiles=np.array(src.execute('SELECT x,y FROM tiles WHERE z=14').fetchall())
 x=tiles[:,0];y=tiles[:,1];xs=[];ys=[]
 for ox in [0,.5,1]:
  for oy in [0,.5,1]:
   lon=(x+ox)/16384*360-180;lat=np.degrees(np.arctan(np.sinh(np.pi*(1-2*(y+oy)/16384))));a,b=project(lon,lat);xs.append(a);ys.append(b)
 lo_x=np.min(xs,axis=0);hi_x=np.max(xs,axis=0);lo_y=np.min(ys,axis=0);hi_y=np.max(ys,axis=0)
 for spec in definitions():
  width=spec['widthMeters'];rows=set()
  for a,b,c,d in zip(np.floor(lo_x/width).astype(int),np.floor(hi_x/width).astype(int),np.floor(lo_y/width).astype(int),np.floor(hi_y/width).astype(int)):
   rows.update((spec['level'],xx,yy) for xx in range(a,b+1) for yy in range(c,d+1))
  db.executemany('INSERT OR IGNORE INTO jobs(level,x,y,state) VALUES(?,?,?,0)',sorted(rows));db.commit();print('目録',spec['name'],len(rows),flush=True)

def run(args):
 output=Path(args.output).resolve();source=Path(args.source).resolve();output.parent.mkdir(exist_ok=True,parents=True)
 progress=output.with_suffix('.progress.json');started=time.time();completed=0;last_status=0
 if shutil.disk_usage(output.parent).free<args.min_free_gib*2**30:raise ValueError('空き容量が開始条件未満です')
 config=dict(schemaVersion=1,levels=definitions(),source=str(source),sourceSize=source.stat().st_size,sourceMtimeNs=source.stat().st_mtime_ns,projection=dict(type='spherical Lambert conformal conic',radius=R,standardParallels=[30,46],origin=[138,36]),sampling='L14 bilinear; grid129,257,513,1025 capped; missing stored as mask; empty tiles omitted',hints='int16x2, scale8; boundary slopes fixed; fits sampled grid up to129; no DEM vertex normals')
 fingerprint=hashlib.sha256(json.dumps(config,sort_keys=True).encode()).hexdigest()
 with sqlite3.connect(output) as db:
  db.execute('PRAGMA journal_mode=WAL');db.execute('PRAGMA synchronous=NORMAL')
  db.executescript('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE IF NOT EXISTS jobs(level INTEGER,x INTEGER,y INTEGER,state INTEGER,error TEXT,PRIMARY KEY(level,x,y));CREATE TABLE IF NOT EXISTS tiles(level INTEGER,x INTEGER,y INTEGER,geometry BLOB,hints BLOB,mask BLOB,stats TEXT,PRIMARY KEY(level,x,y));')
  db.execute('CREATE INDEX IF NOT EXISTS pending_jobs ON jobs(state,level DESC,y,x)')
  prior=db.execute('SELECT value FROM metadata WHERE key="fingerprint"').fetchone()
  if prior and prior[0]!=fingerprint:raise ValueError('生成条件または元データが変わっています。別の出力先が必要です')
  db.execute('INSERT OR REPLACE INTO metadata VALUES("fingerprint",?)',(fingerprint,));db.execute('INSERT OR REPLACE INTO metadata VALUES("manifest",?)',(json.dumps(config),));db.execute('INSERT OR REPLACE INTO metadata VALUES("enabledLevels",?)',(json.dumps(args.levels),));db.commit();catalog(db,source)
  def status(state,force=False):
   nonlocal last_status
   if not force and time.time()-last_status<10:return
   last_status=time.time()
   rows=db.execute('SELECT level,state,count(*) FROM jobs GROUP BY level,state').fetchall();sizes=db.execute('SELECT level,count(*),sum(length(geometry)),sum(length(hints)),sum(length(mask)) FROM tiles GROUP BY level').fetchall()
   remaining=sum(count for _,s,count in rows if s==0);payload=dict(state=state,updated=time.time(),elapsedSeconds=time.time()-started,completedThisRun=completed,estimatedRemainingSeconds=remaining*(time.time()-started)/completed if completed else None,jobs=rows,sizes=sizes,databaseBytes=output.stat().st_size,walBytes=Path(str(output)+'-wal').stat().st_size if Path(str(output)+'-wal').exists() else 0,freeBytes=shutil.disk_usage(output.parent).free)
   tmp=progress.with_suffix('.tmp');tmp.write_text(json.dumps(payload,indent=2),encoding='utf-8');tmp.replace(progress)
  if args.retry:db.execute('UPDATE jobs SET state=0,error=NULL WHERE state=3');db.commit()
  query='SELECT level,x,y FROM jobs WHERE state=0'+(' AND level IN ('+','.join(map(str,args.levels))+')' if args.levels else '')+' ORDER BY level DESC,y,x'
  cursor=db.execute(query);limited=0
  def next_job():
   nonlocal limited
   if args.limit and limited>=args.limit:return None
   row=cursor.fetchone();limited+=bool(row);return row
  state='running';status(state)
  with ProcessPoolExecutor(max_workers=args.workers,initializer=initialize,initargs=(str(source),str(ROOT/'build/terrain-rust-target/release/terrain-adaptive-tin.exe'))) as pool:
   pending={}
   for _ in range(args.workers*2):
    job=next_job()
    if job:pending[pool.submit(generate_tile,job)]=job
   while pending:
    done,_=wait(pending,timeout=10,return_when=FIRST_COMPLETED)
    for future in done:
     job=pending.pop(future)
     try:
      _,result=future.result()
      if result:db.execute('INSERT OR REPLACE INTO tiles VALUES(?,?,?,?,?,?,?)',(*job,*result))
      db.execute('UPDATE jobs SET state=?,error=NULL WHERE level=? AND x=? AND y=?',(1 if result else 2,*job));completed+=1
     except Exception as exc:
      db.execute('UPDATE jobs SET state=3,error=? WHERE level=? AND x=? AND y=?',(str(exc),*job));print('失敗',job,str(exc),flush=True)
     db.commit()
     if shutil.disk_usage(output.parent).free<args.min_free_gib*2**30:state='disk-space-paused'
     job=next_job() if state=='running' else None
     if job:pending[pool.submit(generate_tile,job)]=job
    status(state)
   if state=='running':state='failed' if db.execute('SELECT count(*) FROM jobs WHERE state=3').fetchone()[0] else 'complete' if db.execute('SELECT count(*) FROM jobs WHERE state=0').fetchone()[0]==0 else 'partial'
   cursor.close();db.commit();db.execute('PRAGMA wal_checkpoint(TRUNCATE)');
   if state=='complete':
    validation=verify_database(output,args.levels);db.execute('INSERT OR REPLACE INTO metadata VALUES("verification",?)',(json.dumps(validation),));db.commit()
   status(state,True);print(state,completed,flush=True)

def verify_database(path,levels=None):
 count=0;edges=0
 with open_source(path) as db:
  query='SELECT * FROM tiles'+(' WHERE level IN ('+','.join('?' for _ in levels)+')' if levels else '')
  for level,x,y,geometry,hints,mask,meta in db.execute(query,levels or []):
   raw=zlib.decompress(geometry);magic,n,m,nv,ni=struct.unpack_from('<4sHHII',raw);h=zlib.decompress(hints);v=zlib.decompress(mask)
   if magic!=b'YNT1' or n!=m or not 128<=nv<=1024 or len(raw)!=16+nv*8+ni*2 or ni%3 or len(h)!=12+nv*4 or h[:4]!=b'YNH1' or len(v)!=(n*n+7)//8:raise ValueError(f'形式検証失敗 {(level,x,y)}')
   points=np.array([struct.unpack_from('<HHi',raw,16+i*8) for i in range(nv)]);indices=np.frombuffer(raw,'<u2',ni,16+nv*8).reshape(-1,3)
   if indices.max()>=nv:raise ValueError('頂点参照の範囲外')
   tri=points[indices,:2];u=tri[:,1]-tri[:,0];w=tri[:,2]-tri[:,0];area=u[:,0]*w[:,1]-u[:,1]*w[:,0]
   if np.any(area<=0) or int(area.sum())!=2*(n-1)**2:raise ValueError('形状の反転または面積不一致')
   stats=json.loads(meta)
   for dx,dy,side,other in [(1,0,1,0),(0,1,2,3)]:
    row=db.execute('SELECT stats FROM tiles WHERE level=? AND x=? AND y=?',(level,x+dx,y+dy)).fetchone()
    if row:
     if stats['boundaryDigests'][side]!=json.loads(row[0])['boundaryDigests'][other]:raise ValueError(f'共有辺不一致 {(level,x,y)}')
     edges+=1
   count+=1
 return dict(tiles=count,sharedEdges=edges)

if __name__=='__main__':
 parser=argparse.ArgumentParser();parser.add_argument('--source',default=str(ROOT/'build/terrain-lod.sqlite3'));parser.add_argument('--output',default=str(ROOT/'build/terrain-independent-national.sqlite3'));parser.add_argument('--workers',type=int,default=4);parser.add_argument('--limit',type=int);parser.add_argument('--levels',type=int,nargs='+',choices=range(8),default=[2,3,4,5,6,7]);parser.add_argument('--retry',action='store_true');parser.add_argument('--verify',action='store_true');parser.add_argument('--min-free-gib',type=float,default=3);args=parser.parse_args()
 if args.verify:
  print(json.dumps(verify_database(args.output,args.levels)));raise SystemExit(0)
 pidfile=Path(args.output).resolve().with_suffix('.pid')
 if pidfile.exists():
  pid=pidfile.read_text().strip();listing=subprocess.run(['tasklist','/FI',f'PID eq {pid}','/FO','CSV','/NH'],capture_output=True).stdout.decode(errors='replace')
  if f'"{pid}"' in listing:raise SystemExit('同じ出力先の生成が実行中です')
  pidfile.unlink()
 fd=os.open(pidfile,os.O_CREAT|os.O_EXCL|os.O_WRONLY);os.write(fd,str(os.getpid()).encode());os.close(fd)
 try:run(args)
 finally:pidfile.unlink(missing_ok=True)
