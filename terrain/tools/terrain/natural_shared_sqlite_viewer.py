"""配布サイズ測定用SQLiteだけから制御点を復号・補助情報を再生成して配信する。"""
import argparse,json,sqlite3,subprocess,threading,time,hashlib
from pathlib import Path
from functools import lru_cache
from natural_shared_viewer_server import Dataset as ReferenceDataset,Handler,ViewerHTTPServer,ROOT
class DatabaseDataset(ReferenceDataset):
 def __init__(self,database,cache):
  super().__init__(ROOT/'build/natural-shared-128-v2');self.legacy=ReferenceDataset(ROOT/'build/natural-shared-128-v1');self.database=database.resolve();self.exe=ROOT/'build/natural-fixed-lpf-target/release/terrain-natural-shared-pack.exe'
  with sqlite3.connect(self.database.as_uri()+'?mode=ro',uri=True) as c:
   meta={k:json.loads(v) for k,v in c.execute("select key,value from metadata where key!='scope'")};self._levels=meta['levels'];self.regions={r['region']:r for r in meta['regions']}
   self.counts={(r,e):n for r,e,n in c.execute('select region,error,count(*) from tiles group by region,error')}
  token=hashlib.sha256(self.database.read_bytes()+self.exe.read_bytes()).hexdigest()[:16];self.cache=cache/token;self.cache.mkdir(parents=True,exist_ok=True)
  self.lock=threading.Lock();self.locks={};self.workers=threading.Semaphore(2);self.generated=[];self.cache_hits=0
 def available(self,region):return [v for v in self._levels if self.counts.get((region,v['error']))==(128000//v['tileSide'])**2]
 def manifest(self,region):
  m=super().manifest(region);m['config']['title']='SQLite XYZ復号→Rust補助情報再生成・三地点128 km';return m
 @lru_cache(maxsize=48)
 def packet(self,region,error,mode,x,y):
  if mode=='legacy':return self.legacy.packet(region,error,'partition',x,y)
  if mode!='sqlite':return super().packet(region,error,mode,x,y)
  if error not in [v['error'] for v in self.available(region)]:raise ValueError('未収録品質')
  key=(region,int(error),x,y)
  with self.lock:lock=self.locks.setdefault(key,threading.Lock())
  with lock:
   folder=self.cache/region/str(int(error));folder.mkdir(exist_ok=True,parents=True);out=folder/f'{x}-{y}.bin'
   if not out.exists():
    job=folder/f'{x}-{y}.job.json';job.write_text(json.dumps(dict(database=str(self.database),region=region,error=int(error),x=x,y=y)),encoding='utf-8');start=time.perf_counter()
    with self.workers:
     r=subprocess.run([str(self.exe),'--rebuild','--input',str(job),'--output',str(out)],capture_output=True,text=True,encoding='utf-8',errors='replace',timeout=90)
    if r.returncode:raise RuntimeError(r.stderr[-2000:])
    report=json.loads(out.with_suffix('.json').read_text());report.update(region=region,error=int(error),x=x,y=y,requestSeconds=time.perf_counter()-start)
    with self.lock:self.generated.append(report)
   else:
    with self.lock:self.cache_hits+=1
   return out.read_bytes()
 def status(self):
  with self.lock:return dict(database=str(self.database),sqliteBytes=self.database.stat().st_size,cache=str(self.cache),generatedTiles=len(self.generated),diskCacheHits=self.cache_hits,generated=self.generated)
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--port',type=int,default=61493);p.add_argument('--database',type=Path,default=ROOT/'build/natural-shared-packed-v1/terrain-shared-xyz.sqlite3');p.add_argument('--cache',type=Path,default=ROOT/'build/natural-shared-packed-v1/viewer-cache');a=p.parse_args()
 server=ViewerHTTPServer(('127.0.0.1',a.port),Handler);server.dataset=DatabaseDataset(a.database,a.cache);server.serve_forever()
