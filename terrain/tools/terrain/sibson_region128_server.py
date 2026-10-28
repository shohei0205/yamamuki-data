"""128 km自然近傍の進捗と、必要なタイルだけ評価するローカル配信。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
from collections import OrderedDict
from contextlib import closing
from http.server import ThreadingHTTPServer,BaseHTTPRequestHandler
from pathlib import Path
import json,sqlite3,threading,urllib.parse,struct,traceback
import numpy as np
from scipy.interpolate import CloughTocher2DInterpolator
import time
from sibson_region128 import ROOT,SIZE,TILE,ERRORS
from sibson_shared_codec import decode_shared
from mls_quant_codec import decode_quant
from sibson_surface import SibsonSurface
from sibson_aux_surface import AuxiliarySibsonSurface
from laplace_surface import LaplaceSurface
from spline_natural_comparison import packet as spline_natural_packet
from sibson_delta_codec import checksum
from sibson_pixel import build as build_pixel
from partition_rbf_surface import PartitionRbfSurface

REGIONS=['fuji','yari','tsukuba'];cache=OrderedDict();lock=threading.Lock();slots=threading.Semaphore(2)
pixel_cache=OrderedDict()
aux_models=OrderedDict()
aux_model_lock=threading.Lock()

def cached_aux(region,x,y,error,points):
 import hashlib
 key=(region,x,y,error,hashlib.sha256(points.tobytes()).digest())
 with aux_model_lock:
  if key not in aux_models:
   model=AuxiliarySibsonSurface(points)
   if model.aux_bytes>64*1024*1024:return model
   aux_models[key]=model
   while len(aux_models)>32 or sum(v.aux_bytes for v in aux_models.values())>64*1024*1024:aux_models.popitem(last=False)
  aux_models.move_to_end(key)
  return aux_models[key]

rbf_models=OrderedDict()
rbf_lock=threading.Lock()

def regional_rbf(region,error,db):
 with rbf_lock:
  key=(region,error)
  if key not in rbf_models:
   row=db.execute("SELECT blob FROM models WHERE error=?",(error,)).fetchone()
   if row is None:raise ValueError("地域の共通曲面は生成待ちです")
   rbf_models[key]=PartitionRbfSurface(decode_quant(row[0]))
   while len(rbf_models)>3:rbf_models.popitem(last=False)
  rbf_models.move_to_end(key)
  return rbf_models[key]

def database(region):return closing(sqlite3.connect((ROOT/region/'terrain.sqlite3').resolve().as_uri()+'?mode=ro',uri=True,timeout=30))
def restore(db,x,y,error):
 pe,b,digest=db.execute('SELECT parent_error,blob,sha256 FROM tiles WHERE x=? AND y=? AND error=?',(x,y,error)).fetchone()
 p=decode_quant(b) if pe is None else decode_shared(b,restore(db,x,y,pe),pe)
 if checksum(p)!=digest:raise ValueError('復元検査値が一致しません')
 return p
def field(region,x,y,error,n,method="sibson"):
 key=(region,x,y,error,n,method)
 with lock:
  if key in cache:cache.move_to_end(key);return cache[key]
 with slots,database(region) as db:
  started=time.perf_counter();p=restore(db,x,y,error);model=CloughTocher2DInterpolator(p[:,:2],p[:,2],tol=1e-8,maxiter=400) if method=="ct" else regional_rbf(region,error,db) if method=="rbf" else LaplaceSurface(p) if method=="laplace" else cached_aux(region,x,y,error,p) if method=="sibson-aux" else SibsonSurface(p);step=TILE/n;bounds=np.array([x*TILE,y*TILE,(x+1)*TILE,(y+1)*TILE])
  rect=np.clip(bounds+[-250,-250,250,250],0,SIZE);nx=int(round((rect[2]-rect[0])/step));ny=int(round((rect[3]-rect[1])/step))
  xx,yy=np.meshgrid(np.linspace(rect[0],rect[2],nx+1),np.linspace(rect[1],rect[3],ny+1));q=np.column_stack((xx.ravel(),yy.ravel()));h=model(q)
  gradients=[]
  for axis in range(2):
   a=q.copy();b=q.copy();a[:,axis]=np.minimum(SIZE,a[:,axis]+1);b[:,axis]=np.maximum(0,b[:,axis]-1)
   gradients.append((model(a)-model(b))/(a[:,axis]-b[:,axis]))
  data=np.column_stack((h,*gradients)).astype('<f4').tobytes()
  meta=dict(x=x,y=y,error=error,n=n,nx=nx,ny=ny,step=step,bounds=bounds.tolist(),fieldBounds=rect.tolist(),points=len(p),method=method,evaluationSeconds=time.perf_counter()-started)
  if method=='sibson-aux':meta.update(auxiliaryRamBytes=model.aux_bytes,auxiliaryBuildSeconds=model.aux_build_seconds)
  header=json.dumps(meta).encode();result=struct.pack('<I',len(header))+header+data
 with lock:
  cache[key]=result
  while len(cache)>48 or sum(len(v) for v in cache.values())>192*1024*1024:cache.popitem(last=False)
 return result

class Handler(BaseHTTPRequestHandler):
 def do_GET(self):
  try:
   request=urllib.parse.urlparse(self.path);args=urllib.parse.parse_qs(request.query)
   def arg(k,default=None):return args.get(k,[default])[0]
   region=arg('region','fuji')
   if region not in REGIONS:raise ValueError('不正な地域')
   gpu_assets={'/control-gpu':'sibson_gpu_controls.html','/control-gpu.js':'sibson_gpu_controls.js','/control-gpu.wgsl':'sibson_gpu_controls.wgsl','/control-gpu-render.wgsl':'sibson_gpu_controls_render.wgsl'}
   if request.path in gpu_assets:
    name=gpu_assets[request.path]
    return self.send(200,(Path('tools/terrain')/name).read_bytes(),'text/html; charset=utf-8' if name.endswith('html') else 'text/javascript; charset=utf-8' if name.endswith(('js','mjs')) else 'text/plain; charset=utf-8')
   gpu128_assets={'/control-gpu128':'sibson_gpu_variable.html','/control-gpu128.js':'sibson_gpu_variable.js','/control-gpu128-selection.mjs':'sibson_gpu_variable_selection.mjs','/control-gpu128-render.wgsl':'sibson_gpu_variable_render.wgsl'}
   if request.path in gpu128_assets:
    name=gpu128_assets[request.path]
    return self.send(200,(Path('tools/terrain')/name).read_bytes(),'text/html; charset=utf-8' if name.endswith('html') else 'text/javascript; charset=utf-8' if name.endswith(('js','mjs')) else 'text/plain; charset=utf-8')
   if request.path=='/variable-level-state':
    from sibson_variable_server import database as variable_database
    with variable_database() as db:count=db.execute('SELECT count(*) FROM tiles WHERE region=? AND error=7.25',(region,)).fetchone()[0]
    progress_path=Path('build/sibson-7_25-v1/progress.json');progress=json.loads(progress_path.read_text()) if progress_path.exists() else {}
    return self.send(200,json.dumps(dict(tiles=count,total=16384,progress=progress)).encode(),'application/json')
   if request.path=='/control-gpu128-dem':
    from sibson_gpu_dem import packet as dem_packet
    return self.send(200,dem_packet(region,float(arg('error',800)),int(arg('x',0)),int(arg('y',0)),int(arg('n',128))),'application/octet-stream')
   if request.path=='/control-gpu128-packet':
    from sibson_gpu_variable import packet as gpu128_packet
    return self.send(200,gpu128_packet(region,float(arg('error',800)),int(arg('x',0)),int(arg('y',0))),'application/octet-stream')
   if request.path=='/control-gpu-packet':
    error=float(arg('error',25))
    if error not in [25,50]:raise ValueError('GPU比較の品質が不正です')
    return self.send(200,(Path('build/sibson-weighted-v1/gpu-controls')/f'{region}-{error:g}.bin').read_bytes(),'application/octet-stream')
   if request.path in ['/weighted','/weighted.js']:
    name='sibson_weighted.html' if request.path=='/weighted' else 'sibson_weighted.js'
    return self.send(200,(Path('tools/terrain')/name).read_bytes(),'text/html; charset=utf-8' if name.endswith('html') else 'text/javascript; charset=utf-8')
   if request.path in ['/weighted-field','/weighted-report','/weighted-controls']:
    error=float(arg('error',25));variant=arg('variant','weighted')
    if error not in [25,50] or variant not in ['original','zfit','weighted','extra','extra-zfit','dem','free-bspline','free-bspline-unit','free-bspline-dense','standard-nurbs','standard-nurbs-unit','standard-nurbs-uniform','standard-nurbs-2x','standard-nurbs-4x','natural-precise-shade','natural-fine','sibson-c1','sibson-c1-limited','natural-rounded-small','natural-rounded','natural-global-c1-soft','natural-global-c1']:raise ValueError('比較条件が不正です')
    folder=Path('build/sibson-weighted-v1')/f'{region}-{error:g}'
    return self.send(200,(folder/('report.json' if request.path.endswith('report') else variant+'.controls.json' if request.path.endswith('controls') else variant+'.field')).read_bytes(),'application/json' if request.path.endswith(('report','controls')) else 'application/octet-stream')
   variable_assets={'/variable-wasm.js':'tools/terrain/sibson_variable_wasm.js','/variable-worker.mjs':'tools/terrain/sibson_variable_worker.mjs','/variable-wasm-core.mjs':'tools/terrain/sibson_variable_wasm_core.mjs','/variable-mesh.wasm':'build/sibson_variable_mesh.wasm'}
   if request.path in variable_assets:
    return self.send(200,Path(variable_assets[request.path]).read_bytes(),'application/wasm' if request.path.endswith('.wasm') else 'text/javascript; charset=utf-8')
   if request.path in ['/variable','/variable.js']:
    name='sibson_variable.html' if request.path=='/variable' else 'sibson_variable.js'
    return self.send(200,(Path('tools/terrain')/name).read_bytes(),'text/html; charset=utf-8' if name.endswith('html') else 'text/javascript; charset=utf-8')
   if request.path=='/variable-manifest':
    from sibson_variable_server import manifest as variable_manifest
    return self.send(200,variable_manifest(region),'application/json')
   if request.path=='/variable-field':
    from sibson_variable_server import field as variable_field
    error=float(arg('error'));x=int(arg('x'));y=int(arg('y'));n=int(arg('n',64))
    if error not in ERRORS or not 0<=x<64 or not 0<=y<64 or n not in [16,32,64,128,256,512]:raise ValueError('不正なタイル')
    return self.send(200,variable_field(region,error,x,y,n),'application/octet-stream')
   if request.path in ['/control-patches','/control-patches.js']:
    name='control_patch_comparison.html' if request.path=='/control-patches' else 'control_patch_comparison.js'
    return self.send(200,(Path('tools/terrain')/name).read_bytes(),'text/html; charset=utf-8' if name.endswith('html') else 'text/javascript; charset=utf-8')
   if request.path=='/control-patch-data':
    quality=int(arg('quality',100))
    if quality not in [25,100,400]:raise ValueError('不正な品質')
    return self.send(200,Path(f'build/control-patch-comparison-v1/{region}-{quality}.bin').read_bytes(),'application/octet-stream')
   if request.path=='/spline-natural-data':
    key=('spline-natural',region)
    with lock:body=cache.get(key)
    if body is None:
     with slots:body=spline_natural_packet(region)
     with lock:cache[key]=body
    return self.send(200,body,'application/octet-stream')
   if request.path in ['/sdf-capacity','/sdf-capacity.js']:
    name='terrain_sdf_capacity.html' if request.path=='/sdf-capacity' else 'terrain_sdf_capacity.js'
    return self.send(200,(Path('tools/terrain')/name).read_bytes(),'text/html; charset=utf-8' if name.endswith('html') else 'text/javascript; charset=utf-8')
   if request.path=='/sdf-capacity-data':
    name='resolution27-compare.bin' if arg('size')=='27' else 'capacity-compare.bin'
    return self.send(200,(Path('build/terrain-sdf-v1')/name).read_bytes(),'application/octet-stream')
   if request.path in ['/sdf-fit','/sdf-fit.js']:
    name='terrain_sdf.html' if request.path=='/sdf-fit' else 'terrain_sdf.js'
    return self.send(200,(Path('tools/terrain')/name).read_bytes(),'text/html; charset=utf-8' if name.endswith('html') else 'text/javascript; charset=utf-8')
   if request.path in ['/sdf-data','/sdf-atlas']:
    size=arg('size','128')
    if size not in ['64','128','256','error400']:raise ValueError('不正な解像度')
    extension='png' if request.path=='/sdf-atlas' else 'bin'
    return self.send(200,Path(f'build/terrain-sdf-v1/yari-{size}.{extension}').read_bytes(),'image/png' if extension=='png' else 'application/octet-stream')
   if request.path in ['/loop-fit','/loop-fit.js']:
    name='loop_dem_fit.html' if request.path=='/loop-fit' else 'loop_dem_fit.js'
    return self.send(200,(Path('tools/terrain')/name).read_bytes(),'text/html; charset=utf-8' if name.endswith('html') else 'text/javascript; charset=utf-8')
   if request.path=='/loop-data':
    budget=int(arg('budget',2048))
    if budget not in [512,2048,8192]:raise ValueError('不正な点数')
    return self.send(200,Path(f'build/loop-dem-fit-v1/yari-{budget}.bin').read_bytes(),'application/octet-stream')
   if request.path=='/status':
    status={}
    for r in REGIONS:
     try:status[r]=json.loads((ROOT/r/'progress.json').read_text(encoding='utf-8'))
     except (FileNotFoundError,json.JSONDecodeError):status[r]=dict(stage='waiting')
    return self.send(200,json.dumps(status).encode(),'application/json')
   if request.path=='/manifest':
    rows=[]
    if (ROOT/region/'terrain.sqlite3').exists():
     with database(region) as db:rows=[json.loads(v[0]) for v in db.execute('SELECT info FROM tiles ORDER BY error DESC')]
    return self.send(200,json.dumps(dict(region=region,span=SIZE,tileSize=TILE,tiles=rows)).encode(),'application/json')
   if request.path=='/field':
    x=int(arg('x'));y=int(arg('y'));n=int(arg('n',32));error=float(arg('error'))
    if not 0<=x<16 or not 0<=y<16 or n not in [16,32,64,128,256,512,1024] or error not in ERRORS:raise ValueError('不正なタイル')
    method=arg('method','sibson')
    if method not in ['sibson','ct','rbf','laplace','sibson-aux']:raise ValueError('不正な補間方式')
    return self.send(200,field(region,x,y,error,n,method),'application/octet-stream')
   if request.path=='/gpu':
    x=int(arg('x'));y=int(arg('y'));error=float(arg('error'))
    if not 0<=x<16 or not 0<=y<16 or error not in ERRORS:raise ValueError('不正なタイル')
    key=(region,x,y,error)
    with lock:body=pixel_cache.get(key)
    if body is None:
     with slots,database(region) as db:body=build_pixel(restore(db,x,y,error),[x*TILE,y*TILE,(x+1)*TILE,(y+1)*TILE])
     with lock:
      pixel_cache[key]=body
      while len(pixel_cache)>24:pixel_cache.popitem(last=False)
    return self.send(200,body,'application/octet-stream')
   name={'/':'sibson_region128.html','/viewer.js':'sibson_region128.js','/pixel.js':'sibson_pixel_shader.js','/pixel-viewer.js':'sibson_pixel_viewer.js'}.get(request.path)
   if name:return self.send(200,(Path('tools/terrain')/name).read_bytes(),'text/html; charset=utf-8' if name.endswith('html') else 'text/javascript; charset=utf-8')
   self.send(404,b'not found','text/plain')
  except Exception as e:
   traceback.print_exc();self.send(409,json.dumps(dict(error=str(e))).encode(),'application/json')
 def send(self,status,body,mime):
  self.send_response(status);self.send_header('Content-Type',mime);
  if self.path.startswith('/spline-natural-data'):self.send_header('Access-Control-Allow-Origin','http://127.0.0.1:61473')
  self.send_header('Content-Length',str(len(body)));self.send_header('Cache-Control','no-store');self.end_headers();self.wfile.write(body)
 def log_message(self,*args):pass

if __name__=='__main__':ThreadingHTTPServer(('127.0.0.1',int(os.environ.get('SIBSON128_PORT','61485'))),Handler).serve_forever()
