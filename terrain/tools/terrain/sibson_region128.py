"""128 kmの共通自然近傍曲面を適合し、同品質で一致するタイルへ分ける。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
from pathlib import Path
import concurrent.futures,json,sqlite3,sys,time,hashlib,subprocess,gc
import numpy as np
from scipy.ndimage import gaussian_filter
from scipy.stats import qmc
from ridges import decode_png
from raw_dem_reference import STEP,center_for
from sibson_surface import SibsonSurface
from sibson_tile_seams import extract_points
from mls_quant_codec import encode_quant,decode_quant
from sibson_shared_codec import encode_shared,xy_keys,decode_shared
from sibson_delta_codec import checksum
from sibson_region128_scan import ScanCache

ROOT=Path(os.environ.get('SIBSON128_ROOT','build/sibson-region128-v1'))
ERRORS=[800.,400.,200.,100.,50.,25.,12.5]
SIZE=128000.;TILE=8000.

def write(path,value):
 path.parent.mkdir(parents=True,exist_ok=True)
 tmp=path.with_suffix('.tmp');tmp.write_text(json.dumps(value,ensure_ascii=False,indent=2),encoding='utf-8',newline='\n')
 for attempt in range(100):
  try:tmp.replace(path);return
  except PermissionError:
   if attempt==99:raise
   time.sleep(.05)

class RegionDem:
 def __init__(self,region,half=66000):
  self.center=np.asarray(center_for(region));lo=np.floor((self.center-half)/STEP/256).astype(int);hi=np.floor((self.center+half)/STEP/256).astype(int)
  self.left,self.top=lo*256;self.grid=np.full(tuple(((hi-lo+1)*256)[::-1]),np.nan,np.float32);self.missing=[];self.files=[]
  for y in range(lo[1],hi[1]+1):
   for x in range(lo[0],hi[0]+1):
    p=Path(f'build/terrain-cache/14/{x}/{y}.png')
    if not p.exists():self.missing.append([int(x),int(y)]);continue
    b=p.read_bytes();i=y*256-self.top;j=x*256-self.left;self.grid[i:i+256,j:j+256]=decode_png(b);self.files.append([int(x),int(y),hashlib.sha256(b).hexdigest()])
  self.filtered=None
 def rows(self,rows=16,stride=1):
  lo=np.ceil((self.center-SIZE/2)/STEP-.5).astype(int);hi=np.floor((self.center+SIZE/2)/STEP-.5).astype(int)
  x=np.arange(lo[0],hi[0]+1,stride);xx=(x+.5)*STEP-self.center[0]+SIZE/2
  for first in range(lo[1],hi[1]+1,rows*stride):
   y=np.arange(first,min(first+rows*stride,hi[1]+1),stride);a,b=np.meshgrid(xx,(y+.5)*STEP-self.center[1]+SIZE/2)
   h=self.grid[np.ix_(y-self.top,x-self.left)].ravel();valid=np.isfinite(h)
   yield np.column_stack((a.ravel(),b.ravel()))[valid],h[valid]
 def axes(self):
  lo=np.ceil((self.center-SIZE/2)/STEP-.5).astype(int);hi=np.floor((self.center+SIZE/2)/STEP-.5).astype(int)
  x=np.arange(lo[0],hi[0]+1);y=np.arange(lo[1],hi[1]+1)
  return x,y,(x+.5)*STEP-self.center[0]+SIZE/2,(y+.5)*STEP-self.center[1]+SIZE/2
 def sample(self,q,filtered=False):
  q=np.asarray(q);uv=(q+self.center-SIZE/2)/STEP-.5
  u=uv[:,0]-self.left;v=uv[:,1]-self.top;i=np.floor(u).astype(int);j=np.floor(v).astype(int);a=u-i;b=v-j
  grid=self.filtered if filtered else self.grid;num=np.zeros(len(q));den=np.zeros(len(q))
  for dx,dy,w in [(0,0,(1-a)*(1-b)),(1,0,a*(1-b)),(0,1,(1-a)*b),(1,1,a*b)]:
   values=grid[np.clip(j+dy,0,len(grid)-1),np.clip(i+dx,0,grid.shape[1]-1)];valid=np.isfinite(values)
   num+=np.where(valid,values,0)*w;den+=valid*w
  return np.divide(num,den,out=np.full(len(q),np.nan),where=den>1e-10)
 def filter(self,sigma):
  self.filtered=None
  import gc
  gc.collect()
  valid=np.isfinite(self.grid);weights=valid.astype(np.float32)
  gaussian_filter(weights,sigma/STEP,output=weights,mode='nearest',truncate=4)
  numerator=self.grid.copy()
  for first in range(0,len(numerator),64):
   block=numerator[first:first+64];block[~np.isfinite(block)]=0
  gaussian_filter(numerator,sigma/STEP,output=numerator,mode='nearest',truncate=4)
  np.maximum(weights,1e-12,out=weights);np.divide(numerator,weights,out=numerator);numerator[~valid]=np.nan;self.filtered=numerator
  maximum=0.
  for first in range(0,len(self.grid),64):
   delta=abs(self.filtered[first:first+64]-self.grid[first:first+64]);finite=delta[np.isfinite(delta)]
   if len(finite):maximum=max(maximum,float(finite.max()))
  return maximum

def scan(raw,model,error,progress,stride=1):
 best=np.zeros(len(model.tri));positions=np.zeros((len(best),2));maximum=0.;squared=0.;total=0
 for block,(q,h) in enumerate(raw.rows(stride=stride)):
  err=abs(model(q)-h);maximum=max(maximum,float(err.max(initial=0)));squared+=float(err@err);total+=len(err)
  ids=model.triangulation.find_simplex(q);np.maximum.at(best,ids,err)
  selected=np.flatnonzero((err>=best[ids])&(err>error*.98));positions[ids[selected]]=q[selected]
  if block%16==0:progress(samples=total,measuredMaxError=maximum)
 selected=np.flatnonzero(best>error*.98);selected=selected[np.argsort(-best[selected],kind='stable')[:50000]]
 return maximum,float(np.sqrt(squared/max(1,total))),positions[selected],total

def worker(region,model_only=False,errors=None):
 folder=ROOT/region;folder.mkdir(parents=True,exist_ok=True);started=time.time()
 run_lock=sqlite3.connect(folder/'run-lock.sqlite3',timeout=0)
 run_lock.execute('CREATE TABLE IF NOT EXISTS lock(id INTEGER)');run_lock.commit();run_lock.execute('BEGIN EXCLUSIVE')
 def progress(stage,**values):write(folder/'progress.json',dict(region=region,stage=stage,updatedAt=time.time(),elapsedSeconds=time.time()-started,**values))
 progress('loading-source');raw=RegionDem(region)
 write(folder/'source.json',dict(center=raw.center.tolist(),span=SIZE,sourceStep=STEP,files=raw.files,missingTiles=raw.missing,missingPolicy='欠損は誤差評価対象外。海と断定しない。',coordinateOrigin='地域の左上から東・南へ0..128000 m'))
 db=sqlite3.connect(folder/'terrain.sqlite3');db.execute('PRAGMA journal_mode=WAL')
 db.executescript('CREATE TABLE IF NOT EXISTS models(error REAL PRIMARY KEY,blob BLOB,info TEXT);CREATE TABLE IF NOT EXISTS tiles(x INTEGER,y INTEGER,error REAL,parent_error REAL,blob BLOB,sha256 BLOB,info TEXT,PRIMARY KEY(x,y,error)) WITHOUT ROWID;')
 previous=None;previous_error=None
 for error in (ERRORS if errors is None else errors):
  saved=db.execute('SELECT blob,info FROM models WHERE error=?',(error,)).fetchone()
  if saved:blob,info=saved;info=json.loads(info);points=decode_quant(blob);model=None if model_only else SibsonSurface(points)
  else:
   sigma=error/2
   for attempt in range(10):
    progress('filter-audit',error=error,sigma=sigma);filter_error=raw.filter(sigma)
    if filter_error<=error*.5:break
    sigma*=.5
   else:raise RuntimeError('LPFの誤差条件が未達')
   checkpoint=folder/f'{error:g}-checkpoint.mlq1'
   if checkpoint.exists():xy=decode_quant(checkpoint.read_bytes())[:,:2]
   elif previous is not None:xy=previous[:,:2]
   else:
    xy=qmc.Sobol(2,scramble=True,seed=6021).random_base2(8)*SIZE
    edge=np.linspace(0,SIZE,129);xy=np.unique(np.vstack((xy,np.column_stack((edge,edge*0)),np.column_stack((edge,edge*0+SIZE)),np.column_stack((edge*0,edge)),np.column_stack((edge*0+SIZE,edge)))),axis=0)
   full=False;scan_cache=None
   for iteration in range(128):
    # 欠損位置の外周アンカーのみ仮の0 m。元DEMの有効画素とは区別する。
    model=None;gc.collect();h=np.nan_to_num(raw.sample(xy,True),nan=0.);blob=encode_quant(np.column_stack((xy,h)),xy=.0625,zstep=.125);points=decode_quant(blob);model=SibsonSurface(points)
    callback=lambda **kw:progress('fitting',error=error,iteration=iteration,points=len(points),evaluationStride=1 if full else 4,**kw)
    if full:
     if scan_cache is None:scan_cache=ScanCache(raw)
     maximum,rms,added,total=scan_cache.evaluate(model,points,error,callback)
    else:maximum,rms,added,total=scan(raw,model,error,callback,stride=4)
    checkpoint.write_bytes(blob);info=dict(error=error,points=len(points),maxError=maximum,rmse=rms,samples=total,sigmaMeters=sigma,filterOnlyMaxError=filter_error,iteration=iteration,blobBytes=len(blob))
    progress('iteration-complete',**info)
    if maximum<=error:
     if full:
      audit=ScanCache(raw)
      checked=audit.evaluate(model,points,error,lambda **kw:progress('final-audit',error=error,iteration=iteration,points=len(points),**kw))
      if abs(checked[0]-maximum)>1e-5:raise RuntimeError('局所再検査と独立した全画素検査が一致しません')
      maximum,rms,added,total=checked;info.update(maxError=maximum,rmse=rms,samples=total,independentFullAudit=True)
      if maximum<=error:break
     else:
      full=True
      continue
    updated=np.unique(np.rint(np.vstack((points[:,:2],added))*16)/16,axis=0)
    if len(updated)==len(points):raise RuntimeError('量子化後に追加可能な点がない')
    xy=updated
   else:raise RuntimeError('128反復で最大誤差条件が未達')
   db.execute('INSERT INTO models VALUES(?,?,?)',(error,blob,json.dumps(info)));db.commit()
  if model_only:
   previous=points;previous_error=error;continue
  # 同品質では地域全体の単一曲面から必要な周辺点を取り出す。
  for y in range(16):
   for x in range(16):
    if db.execute('SELECT 1 FROM tiles WHERE x=? AND y=? AND error=?',(x,y,error)).fetchone():continue
    bounds=np.array([x*TILE,y*TILE,(x+1)*TILE,(y+1)*TILE]);rect=np.maximum([0,0,0,0],bounds+[-250,-250,250,250]);rect=np.minimum(rect,SIZE)
    p=extract_points(model,rect);parent=None
    if previous_error is not None:
     row=db.execute('SELECT parent_error,blob FROM tiles WHERE x=? AND y=? AND error=?',(x,y,previous_error)).fetchone()
     def restore(e):
      pe,b=db.execute('SELECT parent_error,blob FROM tiles WHERE x=? AND y=? AND error=?',(x,y,e)).fetchone()
      return decode_quant(b) if pe is None else decode_shared(b,restore(pe),pe)
     parent=restore(previous_error)
     keys=xy_keys(points);order=np.argsort(keys);pk=xy_keys(parent);where=np.searchsorted(keys[order],pk)
     if np.any(where>=len(points)) or not np.array_equal(keys[order][where],pk):raise ValueError('親XYが欠落')
     p=np.unique(np.vstack((p,points[order[where]])),axis=0)
    tile_blob=encode_quant(p,xy=.0625,zstep=.125);p=decode_quant(tile_blob);local=SibsonSurface(p)
    # 境界の法線に使う前後の点も含め、共通曲面と照合する。
    rng=np.random.default_rng(y*16+x);q=rng.uniform(rect[:2],rect[2:],(128,2));difference=float(abs(local(q)-model(q)).max())
    if difference>1e-5:raise RuntimeError(f'余白不足 {region} {error} {x},{y}: {difference}')
    independent=error in [800,400,50];payload=tile_blob if independent else encode_shared(p,parent,previous_error)
    core=(p[:,0]>=bounds[0])&(p[:,0]<=bounds[2])&(p[:,1]>=bounds[1])&(p[:,1]<=bounds[3])
    meta=dict(x=x,y=y,error=error,bounds=bounds.tolist(),points=len(p),haloPoints=int((~core).sum()),bytes=len(payload),maxCurveDifference=difference,supportBounds=[*p[:,:2].min(axis=0).tolist(),*p[:,:2].max(axis=0).tolist()])
    db.execute('INSERT INTO tiles VALUES(?,?,?,?,?,?,?)',(x,y,error,None if independent else previous_error,payload,checksum(p),json.dumps(meta)));db.commit()
    progress('packing',error=error,tiles=y*16+x+1,totalTiles=256,**{k:v for k,v in info.items() if k!='error'})
  previous=points;previous_error=error
  levels=[dict(error=e,tiles=n,bytes=b) for e,n,b in db.execute('SELECT error,count(*),sum(length(blob)) FROM tiles GROUP BY error ORDER BY error DESC')]
  write(folder/'manifest.json',dict(region=region,span=SIZE,tileSize=TILE,levels=levels,sourceMissingTiles=len(raw.missing)))
 if model_only:progress('models-complete',errors=ERRORS if errors is None else errors)
 else:progress('complete',levels=levels)
 db.close();run_lock.close()

def main():
 ROOT.mkdir(exist_ok=True)
 for region in ['fuji','yari','tsukuba']:
  p=ROOT/region/'progress.json'
  if not p.exists() or json.loads(p.read_text())['stage']!='complete':write(p,dict(region=region,stage='queued',updatedAt=time.time()))
 def run(region):
  for attempt in range(3):
   with (ROOT/(region+'.log')).open('a',encoding='utf-8') as log:
    p=subprocess.Popen([sys.executable,'-X','utf8',__file__,region],stdout=log,stderr=subprocess.STDOUT)
    write(ROOT/(region+'.launch.json'),dict(pid=p.pid,startedAt=time.time(),attempt=attempt))
    if p.wait()==0:return
   write(ROOT/region/'progress.json',dict(region=region,stage='retrying',attempt=attempt+1,log=str(ROOT/(region+'.log'))))
   time.sleep(5)
  write(ROOT/region/'progress.json',dict(region=region,stage='failed',log=str(ROOT/(region+'.log'))))
  raise RuntimeError(region+'の生成に失敗。保存済み段階から再開できます。')
 with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:list(pool.map(run,['fuji','yari','tsukuba']))
 write(ROOT/'complete.json',dict(stage='complete'))

if __name__=='__main__':
 if len(sys.argv)>1:worker(sys.argv[1])
 else:main()
