"""3地域128 kmを標準XYZ・見直し誤差で再生成する参照パイプライン。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json,math,sqlite3,time,traceback,threading
from pathlib import Path
from concurrent.futures import ProcessPoolExecutor
import numpy as np
from scipy.stats import qmc
import errorfit_full_tile as fit
from streamed_dem_prune import DemReference,reduce_streamed
from natural_c1_128 import ROOT,REGIONS,write,GlobalC1,close_cells,pack
from sibson_native_codec import encode_independent,decode_independent
from sibson_aux_surface import AuxiliarySibsonSurface
from continuous_dem import STEP,tile
WORLD=40075016.68557849
OUT=ROOT/'build/natural-xyz-tight-errors-v1'
SOURCE_DB=ROOT/'build/terrain-national-natural-20261029/coverage-source.sqlite3'
LEVELS=[(6,640),(7,500),(8,400),(9,320),(10,240),(11,160),(12,80),(13,40),(14,20)]
def connect():
 c=sqlite3.connect(OUT/'queue.sqlite3',timeout=60);c.execute('PRAGMA journal_mode=WAL');return c

def bounds(z,x,y):
 side=WORLD/(1<<z);return np.array([x*side,y*side,(x+1)*side,(y+1)*side])

def tile_folder(z,x,y):return OUT/'tiles'/str(z)/str(x)/str(y)

def init():
 OUT.mkdir(parents=True,exist_ok=True)
 regions=[]
 with connect() as c:
  c.executescript("CREATE TABLE IF NOT EXISTS jobs(z INT,x INT,y INT,state TEXT,priority REAL,info TEXT,PRIMARY KEY(z,x,y)) WITHOUT ROWID;CREATE TABLE IF NOT EXISTS region_tiles(region TEXT,z INT,x INT,y INT,PRIMARY KEY(region,z,x,y)) WITHOUT ROWID;")
  for name,lat,lon in REGIONS:
   center=np.array([(lon+180)/360*WORLD,(1-math.asinh(math.tan(math.radians(lat)))/math.pi)/2*WORLD]);area=np.r_[center-64000,center+64000];regions.append(dict(region=name,center=center.tolist(),bounds=area.tolist()))
   for z,error in LEVELS:
    side=WORLD/(1<<z);low=np.floor(area[:2]/side).astype(int);high=np.ceil(area[2:]/side).astype(int)
    for y in range(int(low[1]),int(high[1])):
     for x in range(int(low[0]),int(high[0])):
      b=bounds(z,x,y);distance=np.linalg.norm((b[:2]+b[2:])/2-center)/side;priority=distance if distance<1.5 else 1000+distance
      c.execute("INSERT OR IGNORE INTO jobs VALUES(?,?,?,'pending',?,'{}')",(z,x,y,priority));c.execute('UPDATE jobs SET priority=min(priority,?) WHERE z=? AND x=? AND y=?',(priority,z,x,y));c.execute('INSERT OR IGNORE INTO region_tiles VALUES(?,?,?,?)',(name,z,x,y))
 write(OUT/'settings.json',dict(regions=regions,levels=[dict(z=z,error_m=e,side_m=WORLD/(1<<z),fine_error_m=e/2) for z,e in LEVELS],tileScheme='standard-XYZ-EPSG3857',previewSpan=128000,lpf=False,pointBudget=None,source=str(SOURCE_DB),codec='NQC1 Rice independent for comparison',generator='Python orchestration and native Sibson evaluator; reference experiment, not production Rust',productionReady=False))
 seed=OUT/'seed/xyz/source';seed.mkdir(parents=True,exist_ok=True);write(seed/'region.json',dict(origin=[0,0]))

def prepare_seed(z,x,y,error):
 side=WORLD/(1<<z);b=bounds(z,x,y);world=(b[:2]+b[2:])/2;fine=error/2;seed=OUT/'seed/xyz'/f'{fine:g}'/f'{x}-{y}';seed.mkdir(parents=True,exist_ok=True)
 half=side/2+250;lo=np.floor((world-half)/(STEP*256)).astype(int);hi=np.floor((world+half)/(STEP*256)).astype(int);expected=int(np.prod(hi-lo+1))
 with sqlite3.connect(SOURCE_DB.resolve().as_uri()+'?mode=ro',uri=True) as c:
  rows=c.execute('SELECT x,y,state FROM cells WHERE x>=? AND x<=? AND y>=? AND y<=?',(int(lo[0]),int(hi[0]),int(lo[1]),int(hi[1]))).fetchall()
 for tx,ty,state in rows:
  if state=='cached' and not (ROOT/f'build/terrain-cache/14/{tx}/{ty}.png').exists():raise ValueError(f'入力記録はcachedだがPNGが無い: {tx}/{ty}')
 coverage=dict(recorded=len(rows),expected=expected,unrecordedOutsideVerifiedDomain=expected-len(rows),cached=sum(r[2]=='cached' for r in rows),verified404=sum(r[2]=='verified404' for r in rows))
 info=dict(region='xyz',error=fine,x=x,y=y,side=side,origin=world.tolist(),coreBounds=b.tolist(),worldFieldBounds=(b+[-250,-250,250,250]).tolist(),bounds=[-half,-half,half,half],halo=500,sourceCoverage=coverage)
 write(seed/'packet.json',info)
 if not (seed/'control-gradient.npz').exists():
  p=np.array([[-4*side,-4*side,0],[4*side,-4*side,0],[-4*side,4*side,0],[4*side,4*side,0],[-side/4,-side/4,0],[side/4,-side/4,0],[-side/4,side/4,0],[side/4,side/4,0.]])
  np.savez_compressed(seed/'control-gradient.npz',points=p)
 return info,coverage

def build(z,x,y):
 error=dict(LEVELS)[z];fine=error/2;dst=tile_folder(z,x,y);dst.mkdir(parents=True,exist_ok=True);started=time.monotonic()
 if (dst/'selection.json').exists():
  saved=json.loads((dst/'selection.json').read_text())
  if saved.get('quantizedControls') or saved.get('empty'):return saved
  (dst/'selection.json').replace(dst/'selection.pre-quantization.json')
 info,coverage=prepare_seed(z,x,y,error);write(dst/'progress.json',dict(stage='fine-fit',z=z,x=x,y=y,error=error,fineError=fine,started=time.time()))
 fit.BASE=OUT/'seed';fit.OUT=OUT/'fine'
 try:fit.build('xyz',fine,x,y,source_error=fine,seed_policy='dem-corners',max_iterations=20000,pack_depth=16,controls_only=True)
 except ValueError as ex:
  if str(ex)!='有効DEM点がありません':raise
  if coverage['unrecordedOutsideVerifiedDomain']:raise ValueError('有効DEMなし・未確認入力あり。海とは判定しない') from ex
  out=dict(info,error=error,empty=True,verifiedNoData=True);write(dst/'packet.json',out);audit=dict(z=z,x=x,y=y,errorTarget=error,empty=True,finalPoints=0,max=0,allDemPixelsAudited=True,seconds=time.monotonic()-started);write(dst/'selection.json',audit);return audit
 src=fit.OUT/'xyz'/f'{fine:g}'/f'{x}-{y}';points=np.load(src/'control-gradient.npz')['points'];side=info['side'];world=np.asarray(info['origin']);points[:,:2]=np.floor((points[:,:2]+world)/.0625+.5)*.0625-world;points[:,2]=np.floor(points[:,2]/.125+.5)*.125;protected=np.any(abs(points[:,:2])>=side/2-STEP,axis=1);protected[:4]=True
 reference=DemReference(world,side/2+250);active=None
 if (dst/'prune-state.npz').exists():active=np.load(dst/'prune-state.npz')['active']
 last=[0.]
 def progress(**v):
  mask=v.pop('active',None)
  if mask is not None:
   temp=dst/'prune-state.tmp.npz';np.savez_compressed(temp,active=mask);temp.replace(dst/'prune-state.npz')
  if time.monotonic()-last[0]>.5 or v.get('stage')=='final-full-audit':
   last[0]=time.monotonic();write(dst/'progress.json',dict(z=z,x=x,y=y,error=error,fineError=fine,totalSeconds=time.monotonic()-started,**v))
 if (dst/'pruned-ready.json').exists() and json.loads((dst/'pruned-ready.json').read_text()).get('quantizedControls'):
  mask=np.load(dst/'prune-state.npz')['active'];model=AuxiliarySibsonSurface(points[mask]);audit=json.loads((dst/'pruned-ready.json').read_text())
 else:
  try:mask,model,audit=reduce_streamed(points,reference,error,protected,progress,active)
  except ValueError as ex:
   if active is None or not str(ex).startswith('削減元が誤差超過'):raise
   mask,model,audit=reduce_streamed(points,reference,error,protected,progress,None)
  audit['quantizedControls']=True;np.savez_compressed(dst/'prune-state.npz',active=mask);write(dst/'pruned-ready.json',audit)
 p=points[mask];field=np.asarray(info['bounds']);n=max(16384,min(262144,2**int(np.ceil(np.log2(len(p)*4)))));q=qmc.Sobol(2,scramble=True,seed=9124).random_base2(int(np.log2(n)))*(field[2:]-field[:2])+field[:2]
 progress(stage='C1-and-GPU-pack',points=len(p)-4);c1=GlobalC1(p,strength=.01,queries=q,surface=model);close_cells(c1.fast_model)
 meta=dict({k:v for k,v in info.items() if k not in ['bounds','error']},error=error,curve='XYZ-Sibson-error-tested-pruning',lpf=False,sourceGeometryBytes=len(p)*12,maxErrorGuaranteed=False,allDemPixelsAudited=True,auditedMaxError=audit['max'],auditedErrorTarget=error,auditScope='all valid available source DEM pixel centers in tile plus 250m band; unrecorded source domain excluded; C1/render/blending excluded')
 packet,out=pack(p,c1.gradient,field,meta,model=c1.fast_model,max_candidates=48,max_depth=16)
 if out['unsafeLeaves']:raise ValueError('unsafe Voronoi leaves')
 origin=np.floor(bounds(z,x,y)[:2]/.0625+.5).astype(np.int64);integers=np.empty(p.shape,np.int64);integers[:,:2]=np.floor((p[:,:2]+world)/.0625+.5).astype(np.int64)-origin;integers[:,2]=np.floor(p[:,2]/.125+.5).astype(np.int64);blob=encode_independent(integers);assert np.array_equal(integers,decode_independent(blob))
 audit.update(z=z,x=x,y=y,errorTarget=error,fineError=fine,fixedPoints=int(protected.sum())-4,sourceCoverage=coverage,riceBytes=len(blob),gpuBytes=len(packet),seconds=time.monotonic()-started)
 (dst/'packet.bin').write_bytes(packet);(dst/'control-xyz.nqc1').write_bytes(blob);np.savez_compressed(dst/'control-gradient.npz',points=p,gradient=c1.gradient);out['pruningAudit']=audit;write(dst/'packet.json',out);write(dst/'selection.json',audit);write(dst/'progress.json',dict(stage='complete',**audit));return audit

def worker(group):
 while True:
  with connect() as c:
   c.execute('BEGIN IMMEDIATE');where='z>=12' if group=='fine' else 'z<12';order='z DESC' if group!='coarse-root' else 'z ASC'
   row=c.execute(f"SELECT z,x,y FROM jobs WHERE state='pending' AND {where} ORDER BY priority,{order},y,x LIMIT 1").fetchone()
   if row is None:row=c.execute("SELECT z,x,y FROM jobs WHERE state='pending' ORDER BY priority,z DESC,y,x LIMIT 1").fetchone()
   if row is None:return
   c.execute("UPDATE jobs SET state='running' WHERE z=? AND x=? AND y=?",row);c.commit()
  try:result=build(*row);state='ready';print('ready',row,result.get('finalPoints'),flush=True)
  except Exception:state='failed';result=dict(message=traceback.format_exc());print('FAILED',row,result,flush=True)
  with connect() as c:c.execute('UPDATE jobs SET state=?,info=? WHERE z=? AND x=? AND y=?',(state,json.dumps(result),*row))

def snapshot():
 with connect() as c:
  states=dict(c.execute('SELECT state,count(*) FROM jobs GROUP BY state'));levels=[dict(z=z,error=dict(LEVELS)[z],state=s,count=n) for z,s,n in c.execute('SELECT z,state,count(*) FROM jobs GROUP BY z,state')];active=[]
  for z,x,y in c.execute("SELECT z,x,y FROM jobs WHERE state='running'"):
   path=tile_folder(z,x,y)/'progress.json'
   try:a=json.loads(path.read_text()) if path.exists() else {}
   except (OSError,ValueError):a={}
   if a.get('stage')=='fine-fit':
    fp=OUT/'fine/xyz'/f'{dict(LEVELS)[z]/2:g}'/f'{x}-{y}'/'progress.json'
    try:a['fit']=json.loads(fp.read_text()) if fp.exists() else {}
    except (OSError,ValueError):pass
   active.append(dict(z=z,x=x,y=y,progress=a))
  failures=[dict(z=z,x=x,y=y,info=json.loads(i)) for z,x,y,i in c.execute("SELECT z,x,y,info FROM jobs WHERE state='failed' LIMIT 20")]
 return dict(states=states,levels=levels,active=active,failures=failures,total=sum(states.values()),complete=states.get('ready',0)==sum(states.values()),updated=time.time())

def run():
 init();lock=sqlite3.connect(OUT/'run-lock.sqlite3',timeout=0);lock.execute('BEGIN EXCLUSIVE')
 with connect() as c:c.execute("UPDATE jobs SET state='pending' WHERE state='running'")
 stop=threading.Event()
 def monitor():
  while not stop.wait(2):
   try:write(OUT/'progress.json',snapshot())
   except Exception as e:print(e,flush=True)
 t=threading.Thread(target=monitor,daemon=True);t.start();workers=max(1,(os.cpu_count() or 2)//2);groups=['coarse-root','coarse']+['fine']*max(0,workers-2)
 try:
  with ProcessPoolExecutor(max_workers=workers) as pool:
   for future in [pool.submit(worker,g) for g in groups[:workers]]:future.result()
 finally:stop.set();t.join();write(OUT/'progress.json',snapshot())
if __name__=='__main__':run()
