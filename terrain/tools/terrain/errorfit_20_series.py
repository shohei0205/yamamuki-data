"""同じ128km試験領域で誤差20～5120mを比較。旧成果は保持。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json,sqlite3,time,traceback,threading
from pathlib import Path
from concurrent.futures import ProcessPoolExecutor
import errorfit_full_tile as fitter
ROOT=fitter.ROOT
OUT=ROOT/'build/natural-raw-c1-128-errorfit20-full-v1'
COARSE_OUT=ROOT/'build/natural-raw-c1-128-coarse-error-only-v3'
def folder(region,error,x,y):return (COARSE_OUT if error>=1280 else OUT)/region/f'{error:g}'/f'{x}-{y}'
LEVELS=[(5120,128000),(2560,128000),(1280,128000),(640,64000),(320,32000),(160,16000),(80,8000),(40,4000),(20,2000)]
SOURCE={new:old for (new,_),old in zip(LEVELS,[800,800,800,400,200,100,50,25,12.5])}
def connect():
 c=sqlite3.connect(OUT/'queue.sqlite3',timeout=60);c.execute('PRAGMA journal_mode=WAL');return c
def init():
 OUT.mkdir(parents=True,exist_ok=True);COARSE_OUT.mkdir(parents=True,exist_ok=True)
 with connect() as c:
  c.execute('CREATE TABLE IF NOT EXISTS jobs(region TEXT,error REAL,x INT,y INT,priority REAL,state TEXT,info TEXT,PRIMARY KEY(region,error,x,y)) WITHOUT ROWID')
  for r in ['fuji','yari','tsukuba']:
   for level,(e,side) in enumerate(LEVELS):
    n=round(128000/side)
    for y in range(n):
     for x in range(n):
      near=abs(x+.5-n/2)<=1 and abs(y+.5-n/2)<=1
      c.execute("INSERT OR IGNORE INTO jobs VALUES(?,?,?,?,?,'pending','{}')",(r,e,x,y,level if near else 100+level))
def worker():
 fitter.OUT=OUT
 while True:
  with connect() as c:
   c.execute('BEGIN IMMEDIATE');row=c.execute("SELECT region,error,x,y FROM jobs WHERE state='pending' ORDER BY priority,region,y,x LIMIT 1").fetchone()
   if row is None:return
   c.execute("UPDATE jobs SET state='running' WHERE region=? AND error=? AND x=? AND y=?",row);c.commit()
  try:
   fitter.OUT=COARSE_OUT if row[1]>=1280 else OUT
   info=fitter.build(*row,source_error=SOURCE[row[1]],seed_policy="dem-corners" if row[1]>=1280 else "legacy");state='ready';print('ready',row,flush=True)
  except Exception:state='failed';info=dict(message=traceback.format_exc());print('FAILED',row,info,flush=True)
  with connect() as c:c.execute('UPDATE jobs SET state=?,info=? WHERE region=? AND error=? AND x=? AND y=?',(state,json.dumps(info),*row))
def snapshot():
 with connect() as c:
  states=dict(c.execute('SELECT state,count(*) FROM jobs GROUP BY state'))
  levels=[dict(region=r,error=e,state=s,count=n) for r,e,s,n in c.execute('SELECT region,error,state,count(*) FROM jobs GROUP BY region,error,state')]
  active=[]
  for r,e,x,y in c.execute("SELECT region,error,x,y FROM jobs WHERE state='running'"):
   path=folder(r,e,x,y)/'progress.json'
   try:record=json.loads(path.read_text()) if path.exists() else {}
   except (OSError,ValueError):record={}
   active.append(dict(region=r,error=e,x=x,y=y,progress=record))
  failed=[dict(region=r,error=e,x=x,y=y,info=json.loads(i)) for r,e,x,y,i in c.execute("SELECT region,error,x,y,info FROM jobs WHERE state='failed' LIMIT 10")]
 return dict(total=sum(states.values()),states=states,levels=levels,active=active,failed=failed,complete=states.get('ready',0)==sum(3*round(128000/s)**2 for _,s in LEVELS),updated=time.time(),tileScheme='comparison-fixed-km-not-XYZ')
def run():
 init();lock=sqlite3.connect(OUT/'run-lock.sqlite3',timeout=0);lock.execute('BEGIN EXCLUSIVE')
 with connect() as c:c.execute("UPDATE jobs SET state='pending' WHERE state='running'")
 stop=threading.Event()
 def monitor():
  while not stop.is_set():
   try:fitter.write(OUT/'progress.json',snapshot())
   except Exception as e:print(e,flush=True)
   stop.wait(3)
 thread=threading.Thread(target=monitor,daemon=True);thread.start()
 try:
  workers=max(1,(os.cpu_count() or 2)//2)
  with ProcessPoolExecutor(max_workers=workers) as pool:
   for f in [pool.submit(worker) for _ in range(workers)]:f.result()
 finally:stop.set();thread.join();fitter.write(OUT/'progress.json',snapshot())
if __name__=='__main__':run()
