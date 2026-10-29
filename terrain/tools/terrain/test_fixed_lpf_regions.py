"""固定LPFとLPF基準適合の3地域試験。旧生成物とは別世代へ保存する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
from pathlib import Path
from contextlib import closing
from concurrent.futures import ThreadPoolExecutor
import argparse,json,math,sqlite3,subprocess,threading,time,shutil,struct
import numpy as np
from scipy.ndimage import map_coordinates
from PIL import Image
import ctypes
_ZSTD_PATH=next((Path(__file__).resolve().parents[2]/'build/terrain-venv/Lib/site-packages/rasterio.libs').glob('zstd-*.dll'))
_ZSTD=ctypes.CDLL(str(_ZSTD_PATH));_ZSTD.ZSTD_getFrameContentSize.argtypes=[ctypes.c_void_p,ctypes.c_size_t];_ZSTD.ZSTD_getFrameContentSize.restype=ctypes.c_ulonglong
_ZSTD.ZSTD_decompress.argtypes=[ctypes.c_void_p,ctypes.c_size_t,ctypes.c_void_p,ctypes.c_size_t];_ZSTD.ZSTD_decompress.restype=ctypes.c_size_t
_ZSTD.ZSTD_isError.argtypes=[ctypes.c_size_t];_ZSTD.ZSTD_isError.restype=ctypes.c_uint
def decompress(blob):
 src=ctypes.create_string_buffer(blob);n=_ZSTD.ZSTD_getFrameContentSize(src,len(blob))
 if n==18446744073709551615:n=16+4*1024*1024
 if n>64*1024*1024:raise ValueError('LPFキャッシュの展開サイズが不正です')
 dst=ctypes.create_string_buffer(n);actual=_ZSTD.ZSTD_decompress(dst,n,src,len(blob))
 if _ZSTD.ZSTD_isError(actual) or actual!=n:raise ValueError('LPFキャッシュを展開できません')
 return dst.raw
ROOT=Path(__file__).resolve().parents[2];OUT=ROOT/'build/natural-fixed-lpf-regions-v1';STEP=40075016.68557849/4194304
REGIONS=[('fuji',35.3606,138.7274),('yari',36.3419,137.6476),('tsukuba',36.2254,140.1067)]
LEVELS=[(8,800),(9,400),(10,200),(11,100),(12,50),(13,25),(14,12.5)]
def write(p,v):
 # Windowsの読取ハンドルやウイルススキャンとの短時間の競合に耐える。
 import tempfile
 p=Path(p)
 with tempfile.NamedTemporaryFile(mode='w',encoding='utf-8',dir=p.parent,prefix=p.name+'.',suffix='.tmp',delete=False) as f:
  json.dump(v,f,ensure_ascii=False,indent=2);t=Path(f.name)
 try:
  for attempt in range(40):
   try:t.replace(p);return
   except PermissionError:
    if attempt==39:raise
    time.sleep(min(.01*(attempt+1),.1))
 finally:
  t.unlink(missing_ok=True)
def preview(output,name,z,error,center,engine):
 target=output/f'preview-{error:g}.json'
 fit_ratio=json.loads((output/'settings.json').read_text()) .get('fit_error_ratio',1.)
 if target.exists():return
 n=512;xs=np.linspace(center[0]-8000,center[0]+8000,n+1);ys=np.linspace(center[1]-8000,center[1]+8000,n+1);xx,yy=np.meshgrid(xs,ys);q=np.column_stack((xx.ravel(),yy.ravel()))
 with closing(sqlite3.connect((output/'work.sqlite3').as_uri()+'?mode=ro',uri=True)) as c:
  pts=np.asarray(c.execute('SELECT p.qx,p.qy,l.qz FROM points p JOIN point_levels l ON p.id=l.id WHERE l.z=? ORDER BY p.id',(z,)).fetchall(),dtype=np.float64)*[.0625,.0625,.125]
  rows=c.execute("SELECT x,y,path FROM artifacts WHERE stage='lpf' AND z=?",(z,)).fetchall()
 lo=np.min([[x,y] for x,y,_ in rows],axis=0);hi=np.max([[x,y] for x,y,_ in rows],axis=0);shape=((hi[1]-lo[1]+1)*1024,(hi[0]-lo[0]+1)*1024);raster=np.full(shape,np.nan,dtype=np.float32)
 for x,y,path in rows:
  blob=decompress(Path(path).read_bytes());values=np.frombuffer(blob,offset=16,dtype='<f4').reshape(1024,1024);raster[(y-lo[1])*1024:(y-lo[1]+1)*1024,(x-lo[0])*1024:(x-lo[0]+1)*1024]=values
 coords=np.array([q[:,1]/STEP-.5-lo[1]*1024,q[:,0]/STEP-.5-lo[0]*1024]);lpf=map_coordinates(raster,coords,order=1,mode='nearest',prefilter=False).reshape(n+1,n+1)
 inp=output/f'evaluate-z{z}.json';res=output/f'evaluate-z{z}-result.json';write(inp,dict(points=pts.tolist(),queries=q.tolist()));subprocess.run([str(engine),'--evaluate',str(inp),'--output',str(res)],cwd=ROOT,check=True);fit=np.asarray(json.loads(res.read_text())).reshape(n+1,n+1)
 raw=np.full(shape,np.nan,dtype=np.float32)
 with closing(sqlite3.connect((output/'work.sqlite3').as_uri()+'?mode=ro',uri=True)) as c:tiles=c.execute('SELECT x,y FROM inputs').fetchall()
 for x,y in tiles:
  rgb=np.asarray(Image.open(ROOT/f'build/terrain-cache/14/{x}/{y}.png')).astype(np.int32);v=(rgb[:,:,0]<<16)|(rgb[:,:,1]<<8)|rgb[:,:,2];h=np.where(v>=8388608,v-16777216,v).astype(np.float32)*.01;h[v==8388608]=np.nan;ox=x*256-lo[0]*1024;oy=y*256-lo[1]*1024;raw[oy:oy+256,ox:ox+256]=h
 dem=map_coordinates(raw,coords,order=1,mode='nearest',prefilter=False).reshape(n+1,n+1)
 for variant,a in [('fit',fit),('lpf',lpf),('dem',dem)]:
  gy,gx=np.gradient(a,16000/n);np.stack([a,gx,gy],axis=-1).astype('<f4').tofile(output/f'{error:g}-{variant}.bin')
 valid=np.isfinite(lpf)&np.isfinite(dem);write(target,dict(region=name,z=z,error=error,sigma=error/2,fit_error_m=(None if json.loads((output/'settings.json').read_text()).get('placement_policy') else error*fit_ratio),points=len(pts)-4,preview_samples=int(valid.sum()),preview_fit_lpf_max=float(abs(fit-lpf)[valid].max()),preview_fit_dem_max=float(abs(fit-dem)[valid].max()),preview_fit_dem_rmse=float(np.sqrt(np.mean((fit-dem)[valid]**2))),preview_lpf_dem_rmse=float(np.sqrt(np.mean((lpf-dem)[valid]**2))),preview_lpf_dem_max=float(abs(lpf-dem)[valid].max()),note=('表示格子上の事後測定。最大誤差の上限保証はしません。' if json.loads((output/'settings.json').read_text()).get('placement_policy') else '表示格子の参考測定。生成時は全有効画素を監査。')));inp.unlink();res.unlink()
def run(span=24000,fit_ratio=1.,output=None,reuse_lpf=None,placement=False):
 global OUT
 if output is not None:OUT=Path(output).resolve()
 OUT.mkdir(parents=True,exist_ok=True);settings=json.loads((ROOT/'NATIONAL_NATURAL_SETTINGS.json').read_text(encoding='utf-8-sig'));settings.update(lpf_policy='fixed-error-half',fit_reference='lpf',fit_error_ratio=fit_ratio,production_ready=False);
 if placement:settings['placement_policy']='lpf-scale-poisson'
 write(OUT/'settings.json',settings)
 engine=OUT/'engine.exe'
 if not engine.exists():shutil.copyfile(ROOT/'build/natural-fixed-lpf-target/release/terrain-natural-national.exe',engine)
 def worker(region):
  name,lat,lon=region;output=OUT/name;output.mkdir(exist_ok=True);center=np.array([(lon+180)/360*40075016.68557849,(1-math.asinh(math.tan(math.radians(lat)))/math.pi)/2*40075016.68557849]);t=center/(STEP*256);half=span/2/(STEP*256);window=[math.floor((t[0]-half)/4)*4,math.floor((t[1]-half)/4)*4,math.ceil((t[0]+half)/4)*4,math.ceil((t[1]+half)/4)*4];write(output/'region.json',dict(region=name,center=center.tolist(),window=window,requested_span=span,preview_span=16000))
  def native(stage,*extra):
   with (output/f'{stage}.stdout.log').open('ab') as a,(output/f'{stage}.stderr.log').open('ab') as b:
    p=subprocess.Popen([str(engine),'--'+stage,'--output',str(output),'--workers','1',*map(str,extra)],cwd=ROOT,stdout=a,stderr=b,creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0));write(output/'launch.json',dict(pid=p.pid,stage=stage,engine=str(engine)))
   if p.wait():raise RuntimeError(f'{name} {stage} failed: {output}')
  native('inventory','--source',ROOT/'build/terrain-lod.sqlite3','--cache',ROOT/'build/terrain-cache','--settings',OUT/'settings.json','--window',*window,*(['--resume'] if (output/'work.sqlite3').exists() else []))
  if reuse_lpf:
   with closing(sqlite3.connect(output/'work.sqlite3')) as dest,closing(sqlite3.connect((Path(reuse_lpf).resolve()/name/'work.sqlite3').as_uri()+'?mode=ro',uri=True)) as src:
    inputs=lambda db:db.execute('SELECT x,y,sha FROM inputs ORDER BY y,x').fetchall()
    if inputs(dest)!=inputs(src):raise RuntimeError('LPF再利用元と元PNGが一致しません')
    with dest:
     for row in src.execute("SELECT * FROM artifacts WHERE stage='lpf'"):dest.execute('INSERT OR REPLACE INTO artifacts VALUES(?,?,?,?,?,?)',row)
     for row in src.execute("SELECT * FROM tasks WHERE stage='lpf'"):dest.execute('INSERT OR REPLACE INTO tasks VALUES(?,?,?,?,?,?)',row)
     for z,e in LEVELS:
      sigma=src.execute('SELECT value FROM metadata WHERE key=?',(f'lpf-sigma-z{z}',)).fetchone()
      if not sigma or float(sigma[0])!=e/2:raise RuntimeError('再利用元のLPF幅が不一致です')
      dest.execute('INSERT OR REPLACE INTO metadata VALUES(?,?)',(f'lpf-sigma-z{z}',sigma[0]))
  native('lpf')
  failure=[];done=threading.Event()
  def monitor():
   while True:
    try:
     with closing(sqlite3.connect((output/'work.sqlite3').as_uri()+'?mode=ro',uri=True,timeout=1)) as c:meta=dict(c.execute('SELECT key,value FROM metadata'))
     for z,e in LEVELS:
      if meta.get(f'fit-phase-z{z}')=='complete':preview(output,name,z,e,center,engine)
    except Exception as ex:failure.append(str(ex));write(output/'preview-error.json',dict(error=str(ex)))
    if done.wait(3):return
  monitor_thread=threading.Thread(target=monitor);monitor_thread.start()
  try:native('place-scale' if placement else 'fit')
  finally:done.set();monitor_thread.join()
  for z,e in LEVELS:preview(output,name,z,e,center,engine)

  if not placement:native('pack-tiles');native('verify')
  write(output/'test-complete.json',dict(complete=True,distribution_complete=False,previews=7))
 with ThreadPoolExecutor(max_workers=3) as pool:list(pool.map(worker,REGIONS))
 write(OUT/'complete.json',dict(complete=True,regions=3,levels=7))
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path);p.add_argument('--fit-ratio',type=float,default=1.);p.add_argument('--reuse-lpf',type=Path);p.add_argument('--placement',action='store_true');args=p.parse_args()
 if not 0<args.fit_ratio<=1:p.error('fit-ratioは0より大きく1以下')
 run(fit_ratio=args.fit_ratio,output=args.output,reuse_lpf=args.reuse_lpf,placement=args.placement)
