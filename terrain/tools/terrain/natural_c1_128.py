"""LPFなし自然近傍+C1、128km全品質のタイル生成。進捗と依頼キューを永続化。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
from pathlib import Path
from contextlib import closing
from concurrent.futures import ThreadPoolExecutor
from functools import lru_cache
import json,sqlite3,subprocess,shutil,time,math,threading,hashlib,traceback
import numpy as np
from scipy.stats import qmc
from test_fixed_lpf_regions import ROOT,write,STEP,REGIONS
from sibson_global_c1 import GlobalC1
from sibson_gpu_controls import pack
from sibson_gpu_variable import close_cells
RUN=ROOT/'build/natural-raw-c1-128-v1'
SOURCE_RUN=RUN
BUDGET_FACTOR=None
ENGINE=RUN/'engine.exe'
LEVELS=[(800,128000),(400,64000),(200,32000),(100,16000),(50,8000),(25,4000),(12.5,2000)]
def connect():
 c=sqlite3.connect(RUN/'queue.sqlite3',timeout=30);c.execute('PRAGMA journal_mode=WAL');return c
def init():
 RUN.mkdir(exist_ok=True)
 with closing(connect()) as c,c:
  c.execute('CREATE TABLE IF NOT EXISTS jobs(region TEXT,error REAL,x INT,y INT,priority REAL,state TEXT,info TEXT,PRIMARY KEY(region,error,x,y)) WITHOUT ROWID')
  for name,_,_ in REGIONS:
   for level,(error,side) in enumerate(LEVELS):
    n=round(128000/side)
    for y in range(n):
     for x in range(n):
      near=(abs(x+.5-n/2)<=1.5 and abs(y+.5-n/2)<=1.5);priority=level if near else 100+level+math.hypot(x+.5-n/2,y+.5-n/2)/n
      c.execute("INSERT OR IGNORE INTO jobs VALUES(?,?,?,?,?,'pending','{}')",(name,error,x,y,priority))
def paths(region,error,x,y):return RUN/region/f'{error:g}'/f'{x}-{y}'
def native(folder,mode,*extra):
 with (folder/f'{mode}.stdout.log').open('ab') as a,(folder/f'{mode}.stderr.log').open('ab') as b:
  args=[str(ENGINE),'--'+mode,'--output',str(folder),'--workers','2',*map(str,extra)];p=subprocess.Popen(args,cwd=ROOT,stdout=a,stderr=b,creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0));write(folder/'launch.json',dict(pid=p.pid,args=args))
 if p.wait():raise RuntimeError(f'{mode} failed: {folder}')
def prepare(region):
 name,lat,lon=region;folder=RUN/name/'source';folder.mkdir(parents=True,exist_ok=True)
 if (folder/'prepared.json').exists():return
 center=np.array([(lon+180)/360*40075016.68557849,(1-math.asinh(math.tan(math.radians(lat)))/math.pi)/2*40075016.68557849]);origin=center-64000;half=82000/(STEP*256);t=center/(STEP*256);window=[math.floor((t[0]-half)/4)*4,math.floor((t[1]-half)/4)*4,math.ceil((t[0]+half)/4)*4,math.ceil((t[1]+half)/4)*4]
 cfg=json.loads((ROOT/'NATIONAL_NATURAL_SETTINGS.json').read_text(encoding='utf-8-sig'));cfg.update(lpf_policy='none',fit_reference='lpf',placement_policy='lpf-scale-poisson',placement_only_z=8);write(folder/'settings-input.json',cfg);write(folder/'region.json',dict(region=name,center=center.tolist(),origin=origin.tolist(),window=window,span=128000))
 native(folder,'inventory','--source',ROOT/'build/terrain-lod.sqlite3','--cache',ROOT/'build/terrain-cache','--settings',folder/'settings-input.json','--window',*window,*(['--resume'] if (folder/'work.sqlite3').exists() else []));native(folder,'lpf');write(folder/'prepared.json',dict(complete=True))
def build(region,error,x,y):
 started=time.perf_counter();target=paths(region,error,x,y);target.mkdir(parents=True,exist_ok=True);parent=SOURCE_RUN/region/'source';r=json.loads((parent/'region.json').read_text());origin=np.asarray(r['origin']);side=dict(LEVELS)[error];core=np.array([x*side,y*side,(x+1)*side,(y+1)*side]);halo=max(500,side/8);domain=core+[-halo,-halo,halo,halo]+np.tile(origin,2);band=250.;field=core+[-band,-band,band,band];worldcenter=origin+(core[:2]+core[2:])/2
 cfg=json.loads((parent/'settings.json').read_text());cfg.update(lpf_cache_blocks=512,placement_quality_m=error,placement_bounds=domain.tolist());
 if BUDGET_FACTOR is not None:
  baseline=json.loads((SOURCE_RUN/region/f'{error:g}'/f'{x}-{y}'/'packet.json').read_text());cfg['fixed_point_budget']=max(4,round((baseline['sourceControls']-4)*BUDGET_FACTOR))
 write(target/'settings.json',cfg)
 dbpath=target/'work.sqlite3'
 if not (target/'points-ready.json').exists():
  with closing(sqlite3.connect(dbpath)) as c,closing(sqlite3.connect((parent/'work.sqlite3').as_uri()+'?mode=ro',uri=True)) as src:
   c.executescript('PRAGMA journal_mode=WAL;CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE IF NOT EXISTS inputs(x INTEGER,y INTEGER,sha TEXT,bytes INTEGER,valid INTEGER,min_cm INTEGER,max_cm INTEGER,PRIMARY KEY(x,y)) WITHOUT ROWID;CREATE TABLE IF NOT EXISTS artifacts(stage TEXT,z INT,x INT,y INT,path TEXT,sha TEXT,PRIMARY KEY(stage,z,x,y)) WITHOUT ROWID;')
   box=(domain+[-2000,-2000,2000,2000])/(STEP*256);lo=np.floor(box[:2]).astype(int);hi=np.ceil(box[2:]).astype(int)
   with c:
    c.executemany('INSERT OR REPLACE INTO inputs VALUES(?,?,?,?,?,?,?)',src.execute('SELECT * FROM inputs WHERE x>=? AND x<? AND y>=? AND y<?',(*[int(lo[0]),int(hi[0]),int(lo[1]),int(hi[1])],)).fetchall())
    c.executemany('INSERT OR REPLACE INTO artifacts VALUES(?,?,?,?,?,?)',src.execute("SELECT * FROM artifacts WHERE stage='lpf' AND z=8 AND x>=? AND x<=? AND y>=? AND y<=?",(int(lo[0]//4),int(hi[0]//4),int(lo[1]//4),int(hi[1]//4))).fetchall());c.execute("INSERT OR REPLACE INTO metadata VALUES('lpf-sigma-z8','0')")
  with closing(sqlite3.connect(dbpath)) as test:valid=test.execute('SELECT coalesce(sum(valid),0) FROM inputs').fetchone()[0]
  if valid==0:
   info=dict(region=region,error=error,x=x,y=y,side=side,origin=((core[:2]+core[2:])/2).tolist(),coreBounds=core.tolist(),worldFieldBounds=field.tolist(),empty=True,seconds=time.perf_counter()-started)
   write(target/'packet.json',info);return info
  native(target,'place-scale');write(target/'points-ready.json',dict(complete=True))
 with closing(sqlite3.connect(dbpath)) as c:rows=c.execute('SELECT p.qx,p.qy,l.qz FROM points p JOIN point_levels l ON l.id=p.id WHERE l.z=8 ORDER BY p.id').fetchall()
 points=np.asarray(rows,dtype=float)*[.0625,.0625,.125];points[:,:2]-=worldcenter
 localfield=field-np.tile((core[:2]+core[2:])/2,2);count=max(16384,2**math.ceil(math.log2(len(points)*4)));count=min(count,262144);q=qmc.Sobol(2,scramble=True,seed=9124).random_base2(int(math.log2(count)));queries=q*(localfield[2:]-localfield[:2])+localfield[:2]
 write(target/'progress.json',dict(stage='global-gradient-fit',points=len(points),samples=len(queries)))
 model=GlobalC1(points,strength=.01,queries=queries);close_cells(model.fast_model)
 write(target/'progress.json',dict(stage='gpu-packet',points=len(points)))
 data,info=pack(points,model.gradient,localfield,dict(region=region,error=error,x=x,y=y,side=side,origin=((core[:2]+core[2:])/2).tolist(),coreBounds=core.tolist(),worldFieldBounds=field.tolist(),curve='Sibson-global-C1-with-halo',lpf=False,sourceGeometryBytes=len(points)*12,gradientScope='tile-plus-halo',halo=halo,maxErrorGuaranteed=False),model=model.fast_model,max_candidates=32,max_depth=8)
 if info['unsafeLeaves']:raise ValueError('GPU補間範囲に未完了のVoronoiセルがあります')
 temp=target/'packet.tmp';temp.write_bytes(data);temp.replace(target/'packet.bin');info.update(seconds=time.perf_counter()-started,gradientFit=model.fit_stats);write(target/'packet.json',info);np.savez_compressed(target/'control-gradient.npz',points=points,gradient=model.gradient)
 return info
def worker():
 while True:
  with closing(connect()) as c:
   c.execute('BEGIN IMMEDIATE');row=c.execute("SELECT region,error,x,y FROM jobs WHERE state='pending' ORDER BY priority,region,error DESC,y,x LIMIT 1").fetchone()
   if row is None:c.commit();return
   c.execute("UPDATE jobs SET state='running' WHERE region=? AND error=? AND x=? AND y=?",row);c.commit()
  try:info=build(*row);state='ready'
  except Exception as e:info=dict(error=str(e),traceback=traceback.format_exc());state='failed'
  with closing(connect()) as c,c:c.execute('UPDATE jobs SET state=?,info=? WHERE region=? AND error=? AND x=? AND y=?',(state,json.dumps(info),*row))
def main():
 global ENGINE
 init()
 release=ROOT/'build/natural-fixed-lpf-target/release/terrain-natural-national.exe';tag=hashlib.sha256(release.read_bytes()).hexdigest()[:16];ENGINE=RUN/f'engine-{tag}.exe'
 if not ENGINE.exists():shutil.copyfile(release,ENGINE)
 write(RUN/'engine-current.json',dict(path=str(ENGINE)))
 with closing(sqlite3.connect(RUN/'run-lock.sqlite3',timeout=0)) as lock:
  lock.execute('BEGIN EXCLUSIVE')
  with ThreadPoolExecutor(max_workers=3) as pool:list(pool.map(prepare,REGIONS))
  with closing(connect()) as c,c:c.execute("UPDATE jobs SET state='pending' WHERE state='running'")
  with ThreadPoolExecutor(max_workers=2) as pool:list(pool.map(lambda _:worker(),range(2)))
  with closing(connect()) as c:counts=dict(c.execute('SELECT state,count(*) FROM jobs GROUP BY state'))
  write(RUN/'complete.json',dict(complete=counts.get('failed',0)==0,counts=counts))
if __name__=='__main__':main()
