"""PNGと過去の404を再利用し、z8内の未確認セルだけ取得元へ確認する。"""
import argparse
from concurrent.futures import ThreadPoolExecutor
from contextlib import closing
from datetime import datetime,timezone
import hashlib,json,sqlite3,time
from pathlib import Path
from fetch_national import fetch_tile,RateLimit


def write(path,value):
 temporary=path.with_suffix('.next.json');temporary.write_text(json.dumps(value,ensure_ascii=False,indent=2)+'\n',encoding='utf-8',newline='\n');temporary.replace(path)


def run(output,records,workers=4,rate=8):
 output=Path(output).resolve();records=Path(records).resolve();started=time.monotonic()
 with closing(sqlite3.connect(output/'coverage-run-lock.sqlite3',timeout=0)) as guard:
  guard.execute('CREATE TABLE IF NOT EXISTS lock(id)');guard.commit();guard.execute('BEGIN EXCLUSIVE')
  with closing(sqlite3.connect((output/'work.sqlite3').as_uri()+'?mode=ro',uri=True)) as source:
   identity=json.loads(source.execute("SELECT value FROM metadata WHERE key='identity'").fetchone()[0]);fingerprint=source.execute("SELECT value FROM metadata WHERE key='input_manifest_sha256'").fetchone()[0];inputs=source.execute('SELECT x,y,sha,bytes,valid FROM inputs ORDER BY y,x').fetchall()
  cache=Path(identity['cache']);roots=sorted({(x>>6,y>>6) for x,y,*_ in inputs});png={(x,y):(sha,size,valid) for x,y,sha,size,valid in inputs}
  with closing(sqlite3.connect(output/'coverage-source.sqlite3')) as db:
   db.executescript("PRAGMA journal_mode=WAL;CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE IF NOT EXISTS cells(x INTEGER,y INTEGER,state TEXT,sha TEXT,bytes INTEGER,checked_at TEXT,url TEXT,error TEXT,PRIMARY KEY(x,y)) WITHOUT ROWID;")
   saved=db.execute("SELECT value FROM metadata WHERE key='input_manifest_sha256'").fetchone()
   if saved and saved[0]!=fingerprint:raise ValueError('入力一覧が変わっています。別の出力を指定してください')
   if not saved:
    known404={};digest=hashlib.sha256()
    with records.open('rb') as f:
     for line in f:
      digest.update(line)
      if not line.strip():continue
      r=json.loads(line);parts=r['url'].split('/')
      if '/xyz/dem_png/14/' not in r['url']:raise ValueError('取得記録の地理院タイル種別が不一致です')
      x,y=int(parts[-2]),int(parts[-1].split('.')[0])
      if r.get('status')==404 and (x>>6,y>>6) in roots:known404[x,y]=(r.get('checkedAt'),r['url'])
    with db:
     db.executemany("INSERT OR IGNORE INTO cells VALUES(?,?,'pending',NULL,0,NULL,NULL,NULL)",((rx*64+dx,ry*64+dy) for rx,ry in roots for dy in range(64) for dx in range(64)))
     db.executemany("UPDATE cells SET state='verified404',checked_at=?,url=? WHERE x=? AND y=?",((date,url,x,y) for (x,y),(date,url) in known404.items() if (x,y) not in png))
     db.executemany("UPDATE cells SET state='cached',sha=?,bytes=? WHERE x=? AND y=?",((sha,size,x,y) for (x,y),(sha,size,_) in png.items()))
     db.execute('INSERT INTO metadata VALUES(?,?)',('input_manifest_sha256',fingerprint));db.execute('INSERT INTO metadata VALUES(?,?)',('archive_sha256',digest.hexdigest()));db.execute('INSERT INTO metadata VALUES(?,?)',('archive_path',str(records)))
   pending=db.execute("SELECT x,y FROM cells WHERE state IN ('pending','error') ORDER BY y,x").fetchall();limiter=RateLimit(rate);processed=0
   def report(stage,error=None):
    states=dict(db.execute('SELECT state,count(*) FROM cells GROUP BY state'));value=dict(stage=stage,states=states,checked_this_run=processed,total_this_run=len(pending),elapsed_seconds=time.monotonic()-started,complete=False,distributable=False,error=error)
    write(output/'coverage-progress.json',value);return value
   report('verifying-source-coverage')
   def fetch(xy):
    x,y=xy
    try:return fetch_tile(x,y,14,cache,limiter),None
    except Exception as error:return (x,y,None,0,None,datetime.now(timezone.utc).isoformat(),None),str(error)
   with ThreadPoolExecutor(max_workers=workers) as pool:
    for begin in range(0,len(pending),32):
     with db:
      for (x,y,status,size,sha,date,url),error in pool.map(fetch,pending[begin:begin+32]):
       db.execute('UPDATE cells SET state=?,sha=?,bytes=?,checked_at=?,url=?,error=? WHERE x=? AND y=?',('error' if error else 'verified404' if status==404 else 'verified200',sha,size,date,url,error,x,y));processed+=1
     report('verifying-source-coverage')
   errors=db.execute("SELECT count(*) FROM cells WHERE state IN ('error','pending')").fetchone()[0]
   new_inputs=[dict(x=x,y=y,sha256=sha,bytes=size) for x,y,sha,size in db.execute("SELECT x,y,sha,bytes FROM cells WHERE state='verified200' ORDER BY y,x")]
   value=report('coverage-input-error' if errors else 'coverage-new-inputs' if new_inputs else 'coverage-inputs-complete');value['new_inputs']=new_inputs;value['source_coverage_verified']=not errors;write(output/'coverage-verification.json',value);db.execute('PRAGMA wal_checkpoint(TRUNCATE)')
   if errors:raise RuntimeError(f'未確認または取得失敗が{errors}枚あります')
   return value


def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--output',type=Path,required=True);p.add_argument('--records',type=Path,required=True);p.add_argument('--workers',type=int,default=4);p.add_argument('--rate',type=float,default=8);a=p.parse_args()
 if not 1<=a.workers<=4 or not 0<a.rate<=8:p.error('同時接続は1～4、頻度は毎秒8以下です')
 print(json.dumps(run(a.output,a.records,a.workers,a.rate),ensure_ascii=False,indent=2))
if __name__=='__main__':main()
