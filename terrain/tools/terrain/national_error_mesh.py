"""全国7品質の基本TINを保存タイルごとに生成する。共通追加前の再開可能な工程。"""
import argparse,hashlib,json,os,shutil,sqlite3,subprocess,tempfile,time,zlib
from pathlib import Path
from concurrent.futures import ProcessPoolExecutor,wait,FIRST_COMPLETED
import numpy as np
from PIL import Image
import national_independent_mesh as national
from error_budget_mesh import base_mesh
ROOT=national.ROOT
ERRORS=[12.5,25,50,100,200,400,800]
WIDTH=8000.;SPACING=31.25;N=257
BINARY=ROOT/'build/terrain-rust-detail20-target/release/terrain-adaptive-tin.exe'
def initialize(source):national.initialize(source,str(BINARY))
def generate(job):
 error,x,y=job
 axis=np.arange(N,dtype=float)*SPACING;xx,yy=np.meshgrid(x*WIDTH+axis,y*WIDTH+axis)
 grid,valid=national.source_samples(xx,yy)
 if not valid.any():return job,None
 grid=np.rint(grid*4)/4;offset=min(0,float(grid.min()))
 with tempfile.TemporaryDirectory(prefix='national-error-base-',dir=ROOT/'build') as temp:
  folder=Path(temp);Image.fromarray(np.rint((grid-offset)*4).astype(np.uint16)).save(folder/'height.png')
  region=dict(folder=folder,grid=grid,height_offset=offset,area=dict(id=f'{error:g}/{x}/{y}'))
  p,f=base_mesh(region,str(BINARY),folder,error=error)
 body=zlib.compress(np.asarray(p,dtype='<f4').tobytes()+np.asarray(f,dtype='<u4').tobytes(),6)
 mask=zlib.compress(np.packbits(valid,bitorder='little').tobytes(),6)
 return job,(len(p),len(f),body,mask)
def run(args):
 output=Path(args.output);progress=output.with_suffix('.progress.json');started=time.time()
 config=dict(version=1,errors=ERRORS,widthMeters=WIDTH,spacingMeters=SPACING,minimumAngleDegrees=10,pruningEnabled=False,additionalFraction=.2,additionalScope='national-global-pool',stage='basic-before-national-additions',source=str(Path(args.source).resolve()),sourceSize=Path(args.source).stat().st_size,sourceMtime=Path(args.source).stat().st_mtime_ns)
 fingerprint=hashlib.sha256(json.dumps(config,sort_keys=True).encode()).hexdigest()
 with sqlite3.connect(output) as db:
  db.execute('PRAGMA journal_mode=WAL');db.execute('PRAGMA synchronous=NORMAL')
  db.executescript('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE IF NOT EXISTS jobs(error REAL,x INTEGER,y INTEGER,state INTEGER DEFAULT 0,failure TEXT,PRIMARY KEY(error,x,y));CREATE TABLE IF NOT EXISTS basic(error REAL,x INTEGER,y INTEGER,vertices INTEGER,triangles INTEGER,mesh BLOB,mask BLOB,PRIMARY KEY(error,x,y));')
  prior=db.execute('SELECT value FROM metadata WHERE key="fingerprint"').fetchone()
  if prior and prior[0]!=fingerprint:raise ValueError('生成条件が異なります。既存データは変更しません')
  db.execute('INSERT OR REPLACE INTO metadata VALUES("fingerprint",?)',(fingerprint,));db.execute('INSERT OR REPLACE INTO metadata VALUES("manifest",?)',(json.dumps(config),))
  if not db.execute('SELECT count(*) FROM jobs').fetchone()[0]:
   with national.open_source(args.catalog) as catalog:keys=catalog.execute('SELECT x,y FROM jobs WHERE level=3').fetchall()
   db.executemany('INSERT INTO jobs(error,x,y) VALUES(?,?,?)',[(e,x,y) for e in ERRORS for x,y in keys])
  db.commit()
  def status(stage):
   state=dict(stage=stage,updated=time.time(),elapsedSeconds=time.time()-started,errors=ERRORS,storageWidthMeters=WIDTH,jobs=db.execute('SELECT error,state,count(*) FROM jobs GROUP BY error,state ORDER BY error').fetchall(),sizes=db.execute('SELECT error,count(*),sum(vertices),sum(triangles),sum(length(mesh)),sum(length(mask)) FROM basic GROUP BY error ORDER BY error').fetchall(),databaseBytes=output.stat().st_size,freeBytes=shutil.disk_usage(output.parent).free,complete=False)
   tmp=progress.with_suffix('.tmp');tmp.write_text(json.dumps(state,ensure_ascii=False,indent=2),encoding='utf-8');tmp.replace(progress)
   if state['freeBytes']<args.min_free_gib*2**30:raise RuntimeError('空き容量の停止条件に達しました。保存済みタイルから再開できます')
  status('basic-generating');keys=db.execute('SELECT error,x,y FROM jobs WHERE state=0 ORDER BY error DESC,y,x').fetchall();keys=keys[:args.limit] if args.limit else keys;iterator=iter(keys)
  with ProcessPoolExecutor(max_workers=args.workers,initializer=initialize,initargs=(args.source,)) as pool:
   pending={}
   def enqueue():
    job=next(iterator,None)
    if job is not None:pending[pool.submit(generate,job)]=job
   for _ in range(args.workers):enqueue()
   while pending:
    done,_=wait(pending,timeout=10,return_when=FIRST_COMPLETED)
    for future in done:
     job=pending.pop(future)
     try:
      _,result=future.result()
      if result:db.execute('INSERT INTO basic VALUES(?,?,?,?,?,?,?)',(*job,*result))
      db.execute('UPDATE jobs SET state=? WHERE error=? AND x=? AND y=?',(1 if result else 2,*job))
     except Exception as exc:
      db.execute('UPDATE jobs SET state=3,failure=? WHERE error=? AND x=? AND y=?',(str(exc),*job));print('基本生成失敗',job,str(exc),flush=True)
     db.commit();enqueue()
    status('basic-generating')
  remaining=db.execute('SELECT count(*) FROM jobs WHERE state IN (0,3)').fetchone()[0]
  status('basic-partial' if remaining else 'basic-ready-for-global-pool')
  print('基本工程終了',dict(remaining=remaining),flush=True)
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--source',default=str(ROOT/'build/terrain-lod.sqlite3'));p.add_argument('--catalog',default=str(ROOT/'build/terrain-independent-national.sqlite3'));p.add_argument('--output',default=str(ROOT/'build/terrain-error-national-base.sqlite3'));p.add_argument('--workers',type=int,default=2);p.add_argument('--min-free-gib',type=float,default=5);p.add_argument('--limit',type=int);a=p.parse_args()
 try:run(a)
 except Exception as exc:
  Path(a.output).with_suffix('.failure.json').write_text(json.dumps(dict(error=str(exc),updated=time.time()),ensure_ascii=False),encoding='utf-8');raise
