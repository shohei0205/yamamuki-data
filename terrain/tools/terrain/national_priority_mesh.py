"""全国の比高優先TINを再開可能なSQLiteへ生成し、共有辺と鋭角面を補修する。"""
import argparse,hashlib,json,os,shutil,sqlite3,struct,subprocess,time,zlib
from pathlib import Path
from concurrent.futures import ProcessPoolExecutor,wait,FIRST_COMPLETED
import numpy as np
from scipy.ndimage import map_coordinates
import national_independent_mesh as national
import regional_priority_levels as regional
import refine_regional_quality as quality
from curve_mesh_refinement import mesh_errors
from national_mesh_levels import ERRORS
ROOT=national.ROOT


def initialize(source,binary):
 os.environ['TERRAIN_CURVE_AWARE']='1'
 national.initialize(source,binary)


def decode(row):
 level,x,y,g,h,m,report=row;raw=zlib.decompress(g);_,n,_,nv,ni=struct.unpack_from('<4sHHII',raw)
 p=np.frombuffer(raw,dtype='<f4',offset=16,count=nv*3).reshape(-1,3).astype(float);p[:,:2]/=1000*2**level/(n-1)
 f=np.frombuffer(raw,dtype='<u2',offset=16+nv*12,count=ni).reshape(-1,3).copy()
 return p.tolist(),f,n,json.loads(report)


def encode(row,points,faces,stats,slopes=None):
 level,x,y,g,h,m,_=row;n=stats['sampleGridSize'];p=np.asarray(points,dtype=float);world=p.copy();world[:,:2]*=1000*2**level/(n-1)
 g=zlib.compress(struct.pack('<4sHHII',b'YNT2',n,n,len(p),faces.size)+world.astype('<f4').tobytes()+faces.astype('<u2').tobytes(),9)
 if slopes is None:
  old=zlib.decompress(h);values=np.frombuffer(old,dtype='<i2',offset=12).reshape(-1,2).copy()
  if len(values)!=len(points):raise ValueError('傾きヒントの点数が一致しません')
 else:values=np.rint(np.clip(slopes,-8,8)/8*32767).astype('<i2')
 h=zlib.compress(struct.pack('<4sIf',b'YNH1',len(p),8.)+values.tobytes(),9)
 edges={}
 for name,axis,value,along in [('north',1,n-1,0),('south',1,0,0),('west',0,0,1),('east',0,n-1,1)]:
  ids=np.flatnonzero(p[:,axis]==value);ids=ids[np.argsort(p[ids,along])];edges[name]=[[int(round(p[i,along])),float(p[i,2])] for i in ids]
 stats.update(vertices=len(p),triangles=len(faces),bytes=len(g)+len(h)+len(m),edges=edges)
 return g,h,m,json.dumps(stats,separators=(',',':'))


def base_job(key):
 key,g,h,m,s=regional.generate_tile(key)
 # 全国の進捗や配布用メタデータへデバッグ用の全追加点を複製しない。
 s.pop('addedPositions',None);s['sampleGridSize']=struct.unpack_from('<H',zlib.decompress(g),4)[0]
 if not np.any(np.frombuffer(zlib.decompress(m),dtype=np.uint8)):return key,None
 s['qualityOriginalBaseVertices']=s['baseVertices'];s['qualityAllowedError']=max(ERRORS[key[0]],s['maxErrorMeters'])
 return key,(g,h,m,json.dumps(s,separators=(',',':')))


def quality_job(row,final=False):
 level,x,y=row[:3];points,faces,n,s=decode(row);grid,limits,gx,gy=quality.source_fields(level,x,y,n)
 existing={tuple(map(int,p[:2])) for p in points};before=len(points);added=s.get('qualityAddedVertices',0)
 quality_budget=min(1024,16384-s['qualityOriginalBaseVertices']);repair_budget=min(2048,16384-s['qualityOriginalBaseVertices'])
 requests=[]
 if not final and added<quality_budget:
  for px,py in quality.quality_candidates(points,faces,n):
   if px in [0,n-1] or py in [0,n-1]:requests.append((x*(n-1)+px,y*(n-1)+py,float(grid[py,px])))
   elif (px,py) not in existing and added<quality_budget:
    points.append([px,py,float(grid[py,px])]);existing.add((px,py));added+=1
 if len(points)!=before:faces=quality.triangulate(points)
 for _ in range(16):
  if added>=repair_budget:break
  candidates=quality.error_candidates(points,faces,grid,s['qualityAllowedError'])
  if not candidates:break
  for px,py in candidates:
   if (px,py) not in existing and added<repair_budget:points.append([px,py,float(grid[py,px])]);existing.add((px,py));added+=1
  faces=quality.triangulate(points)
 p=np.asarray(points);slopes=np.column_stack([map_coordinates(gx,[p[:,1],p[:,0]],order=1),-map_coordinates(gy,[p[:,1],p[:,0]],order=1)])
 # 元の点の傾きは変えず、新しい点だけDEMから補う。
 old=np.frombuffer(zlib.decompress(row[4]),dtype='<i2',offset=12).reshape(-1,2);slopes[:len(old)]=old*8/32767
 s.update(qualityAddedVertices=added,baseVertices=s['qualityOriginalBaseVertices']+added,qualityRefined=True,qualityMinimumAngleTarget=10)
 if final:
  maximum,ratio=mesh_errors(p,faces,grid,limits);s.update(maxErrorMeters=maximum,maxErrorRatio=ratio,toleranceReached=ratio<=1+1e-8,afterUnder10=int((quality.angles(p,faces)<10).sum()))
  if maximum>s['qualityAllowedError']+1e-5:raise ValueError(f'補修後の標高誤差が増加 {(level,x,y)}: {maximum} > {s["qualityAllowedError"]}')
 return encode(row,points,faces,s,slopes),requests,len(points)>before


def boundary_targets(level,gx,gy):
 width=1000*2**level;n=min(2049,max(129,2**int(round(np.log2(width/15.625)))+1));side=n-1
 xs=[gx//side]+([gx//side-1] if gx%side==0 else []);ys=[gy//side]+([gy//side-1] if gy%side==0 else [])
 return [(level,x,y,gx-x*side,gy-y*side) for x in xs for y in ys]


def apply_requests(db,level):
 changed=set();count=0
 for gx,gy,z in db.execute('SELECT gx,gy,z FROM requests WHERE level=? ORDER BY gx,gy',(level,)).fetchall():
  targets=[];blocked=False
  sampleStep=62.5;side=min(2048,max(128,2**int(round(np.log2(1000*2**level/15.625)))));wx=gx*1000*2**level/side;wy=gy*1000*2**level/side
  heights,_=national.source_samples(np.array([wx-sampleStep,wx+sampleStep,wx,wx]),np.array([wy,wy,wy-sampleStep,wy+sampleStep]));edgeSlope=[(heights[1]-heights[0])/(2*sampleStep),-(heights[3]-heights[2])/(2*sampleStep)]
  for l,x,y,px,py in boundary_targets(level,gx,gy):
   row=db.execute('SELECT * FROM tiles WHERE level=? AND x=? AND y=?',(l,x,y)).fetchone()
   if not row:continue
   points,faces,n,s=decode(row)
   if any(int(p[0])==px and int(p[1])==py for p in points):continue
   if s.get('qualityAddedVertices',0)>=min(1024,16384-s['qualityOriginalBaseVertices']):blocked=True;break
   targets.append((row,points,faces,n,s,px,py))
  if blocked:continue
  for row,points,faces,n,s,px,py in targets:
   points.append([px,py,z]);faces=quality.triangulate(points);s['qualityAddedVertices']=s.get('qualityAddedVertices',0)+1;s['baseVertices']=s['qualityOriginalBaseVertices']+s['qualityAddedVertices']
   # 共有点の傾きは両側で同じ座標のDEMから求める。Loopでは傾きヒントを使わない。
   old=np.frombuffer(zlib.decompress(row[4]),dtype='<i2',offset=12).reshape(-1,2)*8/32767;slopes=np.vstack([old,edgeSlope])
   values=encode(row,points,faces,s,slopes);db.execute('UPDATE tiles SET geometry=?,hints=?,mask=?,stats=? WHERE level=? AND x=? AND y=?',(*values,*row[:3]));changed.add(tuple(row[:3]));count+=1
  # 点の両側への追加と進捗を同じトランザクションに保存する。
  for key in changed:db.execute('INSERT OR IGNORE INTO next_active VALUES(?,?,?)',key)
  changed.clear();db.execute('DELETE FROM requests WHERE level=? AND gx=? AND gy=?',(level,gx,gy));db.commit()
 return count


def verify(db,levels):
 edges=0;tiles=0
 for l in levels:
  for row in db.execute('SELECT * FROM tiles WHERE level=? ORDER BY x,y',(l,)):
   p,f,n,s=decode(row);p=np.asarray(p);a=p[f[:,1],:2]-p[f[:,0],:2];b=p[f[:,2],:2]-p[f[:,0],:2];area=a[:,0]*b[:,1]-a[:,1]*b[:,0]
   if np.any(area<=0) or abs(area.sum()-2*(n-1)**2)>1e-3:raise ValueError(f'面積または反転 {(l,*row[1:3])}')
   for dx,dy,side,other in [(1,0,'east','west'),(0,1,'north','south')]:
    r=db.execute('SELECT stats FROM tiles WHERE level=? AND x=? AND y=?',(l,row[1]+dx,row[2]+dy)).fetchone()
    if r:
     if s['edges'][side]!=json.loads(r[0])['edges'][other]:raise ValueError(f'共有辺不一致 {(l,*row[1:3])}')
     edges+=1
   tiles+=1
 return dict(tiles=tiles,sharedEdges=edges)


def run(args):
 output=Path(args.output).resolve();source=Path(args.source).resolve();binary=Path(args.binary).resolve();started=time.time();progress=output.with_suffix('.progress.json')
 initialize(str(source),str(binary))
 with sqlite3.connect(output) as db:
  db.execute('PRAGMA journal_mode=WAL');db.execute('PRAGMA synchronous=NORMAL')
  db.executescript('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE IF NOT EXISTS tiles(level INTEGER,x INTEGER,y INTEGER,geometry BLOB,hints BLOB,mask BLOB,stats TEXT,PRIMARY KEY(level,x,y));CREATE TABLE IF NOT EXISTS jobs(level INTEGER,x INTEGER,y INTEGER,state INTEGER DEFAULT 0,error TEXT,PRIMARY KEY(level,x,y));CREATE TABLE IF NOT EXISTS active(level INTEGER,x INTEGER,y INTEGER,PRIMARY KEY(level,x,y));CREATE TABLE IF NOT EXISTS next_active(level INTEGER,x INTEGER,y INTEGER,PRIMARY KEY(level,x,y));CREATE TABLE IF NOT EXISTS requests(level INTEGER,gx INTEGER,gy INTEGER,z REAL,PRIMARY KEY(level,gx,gy));')
  config=dict(version=1,levels=[s for s in regional.specifications() if s['level'] in args.levels],source=str(source),sourceSize=source.stat().st_size,sourceMtime=source.stat().st_mtime_ns,curveAware=True,priorityMaximumWeight=2,qualityRounds=args.quality_rounds,qualityMinimumAngleTarget=10,demBake=False,projection=dict(type='spherical Lambert conformal conic',radius=national.R,standardParallels=[30,46],origin=[138,36]))
  fingerprint=hashlib.sha256(json.dumps(config,sort_keys=True).encode()).hexdigest();prior=db.execute('SELECT value FROM metadata WHERE key="fingerprint"').fetchone()
  if prior and prior[0]!=fingerprint:raise ValueError('生成条件が異なります。別の出力先を指定してください')
  db.execute('INSERT OR REPLACE INTO metadata VALUES("fingerprint",?)',(fingerprint,));db.execute('INSERT OR REPLACE INTO metadata VALUES("manifest",?)',(json.dumps(config),))
  if not db.execute('SELECT count(*) FROM jobs').fetchone()[0]:
   with national.open_source(args.catalog) as old:
    rows=old.execute('SELECT level,x,y FROM jobs WHERE level IN ('+','.join('?'*len(args.levels))+')',args.levels).fetchall()
   db.executemany('INSERT INTO jobs(level,x,y) VALUES(?,?,?)',rows)
  db.commit()
  def status(stage):
   payload=dict(stage=stage,updated=time.time(),elapsedSeconds=time.time()-started,jobs=db.execute('SELECT level,state,count(*) FROM jobs GROUP BY level,state').fetchall(),active=db.execute('SELECT count(*) FROM active').fetchone()[0],sizes=db.execute('SELECT level,count(*),sum(length(geometry)),sum(length(hints)),sum(length(mask)) FROM tiles GROUP BY level').fetchall(),databaseBytes=output.stat().st_size,freeBytes=shutil.disk_usage(output.parent).free)
   temp=progress.with_suffix('.tmp');temp.write_text(json.dumps(payload,indent=2),encoding='utf-8');temp.replace(progress)
   if payload['freeBytes']<args.min_free_gib*2**30:raise RuntimeError('空き容量が停止条件未満です。保存済みタイルから再開できます')
  if args.verify:print(verify(db,args.levels));return
  if args.retry:db.execute('UPDATE jobs SET state=0,error=NULL WHERE state=3');db.commit()
  with ProcessPoolExecutor(max_workers=args.workers,initializer=initialize,initargs=(str(source),str(binary))) as pool:
   keys=db.execute('SELECT level,x,y FROM jobs WHERE state=0 ORDER BY level DESC,y,x').fetchall();keys=keys[:args.limit] if args.limit else keys;iterator=iter(keys);pending={}
   def enqueue():
    key=next(iterator,None)
    if key:pending[pool.submit(base_job,key)]=key
   for _ in range(args.workers*2):enqueue()
   while pending:
    done,_=wait(pending,timeout=10,return_when=FIRST_COMPLETED)
    for future in done:
     key=pending.pop(future)
     try:
      _,result=future.result()
      if result:db.execute('INSERT OR REPLACE INTO tiles VALUES(?,?,?,?,?,?,?)',(*key,*result))
      db.execute('UPDATE jobs SET state=? WHERE level=? AND x=? AND y=?',(1 if result else 2,*key))
     except Exception as exc:db.execute('UPDATE jobs SET state=3,error=? WHERE level=? AND x=? AND y=?',(str(exc),*key));print('生成失敗',key,str(exc),flush=True)
     db.commit();enqueue()
    status('base')
   if db.execute('SELECT count(*) FROM jobs WHERE state IN (0,3)').fetchone()[0]:status('base-partial');return
   # 各レベルの境界追加を全体で同期してから次の鋭角面補修へ進む。
   for level in sorted(args.levels,reverse=True):
    stageKey=f'quality-L{level}';saved=db.execute('SELECT value FROM metadata WHERE key=?',(stageKey,)).fetchone();roundIndex=int(saved[0]) if saved else 0
    if roundIndex==-1:continue
    if not saved:db.execute('INSERT OR IGNORE INTO active SELECT level,x,y FROM tiles WHERE level=?',(level,));db.commit()
    for iteration in range(roundIndex,args.quality_rounds+1):
     final=iteration==args.quality_rounds
     if final:db.execute('INSERT OR IGNORE INTO active SELECT level,x,y FROM tiles WHERE level=?',(level,));db.commit()
     keys=db.execute('SELECT level,x,y FROM active WHERE level=? ORDER BY y,x',(level,)).fetchall();iterator=iter(keys);pending={}
     def submit():
      key=next(iterator,None)
      if key:
       row=db.execute('SELECT * FROM tiles WHERE level=? AND x=? AND y=?',key).fetchone();pending[pool.submit(quality_job,row,final)]=key
     for _ in range(args.workers*2):submit()
     while pending:
      done,_=wait(pending,timeout=10,return_when=FIRST_COMPLETED)
      for future in done:
       key=pending.pop(future);values,requests,changed=future.result();db.execute('UPDATE tiles SET geometry=?,hints=?,mask=?,stats=? WHERE level=? AND x=? AND y=?',(*values,*key));db.execute('DELETE FROM active WHERE level=? AND x=? AND y=?',key)
       if changed:db.execute('INSERT OR IGNORE INTO next_active VALUES(?,?,?)',key)
       for gx,gy,z in requests:db.execute('INSERT OR IGNORE INTO requests VALUES(?,?,?,?)',(level,gx,gy,z))
       db.commit();submit()
      status(f'quality-L{level}-round{iteration}')
     apply_requests(db,level)
     db.execute('DELETE FROM active WHERE level=?',(level,)) if final else db.execute('INSERT OR IGNORE INTO active SELECT * FROM next_active WHERE level=?',(level,));db.execute('DELETE FROM next_active WHERE level=?',(level,));db.execute('INSERT OR REPLACE INTO metadata VALUES(?,?)',(stageKey,str(-1 if final else iteration+1)));db.commit()
     if final:break
   report=verify(db,args.levels);db.execute('INSERT OR REPLACE INTO metadata VALUES("verification",?)',(json.dumps(report),));db.commit();db.execute('PRAGMA wal_checkpoint(TRUNCATE)');status('complete');print('全国生成完了',report,flush=True)

if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--source',default=str(ROOT/'build/terrain-lod.sqlite3'));p.add_argument('--catalog',default=str(ROOT/'build/terrain-independent-national.sqlite3'));p.add_argument('--output',default=str(ROOT/'build/terrain-priority-national.sqlite3'));p.add_argument('--binary',default=str(ROOT/'build/terrain-rust-target/debug/terrain-adaptive-tin.exe'));p.add_argument('--levels',type=int,nargs='+',default=[2,3,4,5,6,7],choices=range(2,8));p.add_argument('--workers',type=int,default=4);p.add_argument('--quality-rounds',type=int,default=32);p.add_argument('--min-free-gib',type=float,default=5);p.add_argument('--limit',type=int);p.add_argument('--retry',action='store_true');p.add_argument('--verify',action='store_true');a=p.parse_args()
 pidfile=Path(a.output).with_suffix('.pid')
 if pidfile.exists():
  pid=pidfile.read_text().strip();listing=subprocess.run(['tasklist','/FI',f'PID eq {pid}','/FO','CSV','/NH'],capture_output=True).stdout.decode(errors='replace')
  if f'"{pid}"' in listing:raise SystemExit('同じ出力先の生成が実行中です')
 fd=os.open(pidfile,os.O_CREAT|os.O_EXCL|os.O_WRONLY);os.write(fd,str(os.getpid()).encode());os.close(fd)
 try:run(a)
 except Exception as exc:
  failure=Path(a.output).with_suffix('.failure.json');failure.write_text(json.dumps(dict(error=str(exc),updated=time.time()),ensure_ascii=False,indent=2),encoding='utf-8');raise
 finally:pidfile.unlink(missing_ok=True)
