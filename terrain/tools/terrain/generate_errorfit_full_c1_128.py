"""3地域128km・全7品質（16383タイル）の再開可能な誤差適合生成。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json,time,sqlite3,traceback,threading
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path
from errorfit_full_tile import OUT,BASE,build,write
from natural_c1_128 import LEVELS

def connect():
 c=sqlite3.connect(OUT/'queue.sqlite3',timeout=60);c.execute('PRAGMA journal_mode=WAL');return c

def init():
 OUT.mkdir(exist_ok=True)
 with connect() as c:
  c.execute('CREATE TABLE IF NOT EXISTS jobs(region TEXT,error REAL,x INTEGER,y INTEGER,state TEXT,info TEXT,PRIMARY KEY(region,error,x,y)) WITHOUT ROWID')
  for region in ['fuji','yari','tsukuba']:
   for error,side in LEVELS:
    n=round(128000/side)
    for y in range(n):
     for x in range(n):c.execute("INSERT OR IGNORE INTO jobs VALUES(?,?,?,?,'pending','{}')",(region,error,x,y))

def worker():
 while True:
  with connect() as c:
   c.execute('BEGIN IMMEDIATE');row=c.execute("SELECT region,error,x,y FROM jobs WHERE state='pending' ORDER BY error DESC,region,y,x LIMIT 1").fetchone()
   if row is None:c.commit();return
   c.execute("UPDATE jobs SET state='running' WHERE region=? AND error=? AND x=? AND y=?",row);c.commit()
  try:info=build(*row);state='ready';print('ready',row,flush=True)
  except Exception:info=dict(message=traceback.format_exc());state='failed';print('FAILED',row,info['message'],flush=True)
  with connect() as c:c.execute('UPDATE jobs SET state=?,info=? WHERE region=? AND error=? AND x=? AND y=?',(state,json.dumps(info),*row))

def snapshot():
 with connect() as c:
  states=dict(c.execute('SELECT state,count(*) FROM jobs GROUP BY state'));levels=[dict(region=r,error=e,state=s,count=n) for r,e,s,n in c.execute('SELECT region,error,state,count(*) FROM jobs GROUP BY region,error,state')];active=[]
  for region,error,x,y in c.execute("SELECT region,error,x,y FROM jobs WHERE state='running'"):
   path=OUT/region/f'{error:g}'/f'{x}-{y}'/'progress.json';active.append(dict(region=region,error=error,x=x,y=y,progress=json.loads(path.read_text()) if path.exists() else {}))
  failures=[dict(region=r,error=e,x=x,y=y,info=json.loads(i)) for r,e,x,y,i in c.execute("SELECT region,error,x,y,info FROM jobs WHERE state='failed' LIMIT 10")]
 return dict(total=sum(states.values()),states=states,levels=levels,active=active,failed=failures,complete=states.get('ready',0)==16383)

def run():
 init();lock=sqlite3.connect(OUT/'run-lock.sqlite3',timeout=0);lock.execute('BEGIN EXCLUSIVE')
 with connect() as c:c.execute("UPDATE jobs SET state='pending' WHERE state='running'")
 stop=threading.Event()
 def monitor():
  while not stop.is_set():
   try:write(OUT/'progress.json',snapshot())
   except Exception as e:print('progress error',e,flush=True)
   stop.wait(3)
 thread=threading.Thread(target=monitor,daemon=True);thread.start()
 try:
  workers=max(1,(os.cpu_count() or 2)//2)
  print('worker processes',workers,flush=True)
  with ProcessPoolExecutor(max_workers=workers) as pool:
   futures=[pool.submit(worker) for _ in range(workers)]
   for future in futures:future.result()
 finally:stop.set();thread.join();write(OUT/'progress.json',snapshot())
if __name__=='__main__':run()
