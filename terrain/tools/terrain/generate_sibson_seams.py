"""採用SQLiteから自然近傍タイルと接合試験用の表示場を作る。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
from pathlib import Path
import json,sqlite3,sys,subprocess,concurrent.futures
import numpy as np
from sibson_shared_codec import load_shared
from sibson_surface import SibsonSurface
from sibson_tile_seams import TILES,expanded,extract_points,STEP
from mls_quant_codec import encode_quant
ROOT=Path('build/sibson-seams-v1')
VIEW=Path('build/mls-generated-preview/dem-natural-seams')
ERRORS=[800,400,200,100,50,25,12.5]

def worker(region):
 folder=ROOT/region;folder.mkdir(parents=True,exist_ok=True)
 dest=VIEW/region;dest.mkdir(parents=True,exist_ok=True);entries=[]
 with sqlite3.connect(Path('build/sibson-anchor-comparison-v1/depth3.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as db:
  for error in ERRORS:
   points,global_model,_=load_shared(db,region,error)
   for name,bounds in TILES.items():
    rect=expanded(bounds);p=extract_points(global_model,rect);model=SibsonSurface(p)
    rng=np.random.default_rng(633);q=rng.uniform(rect[:2],rect[2:],(8192,2))
    delta=np.max(abs(model(q)-global_model(q)))
    if delta>1e-5:raise ValueError(f'余白の影響範囲が不足: {region} {error} {name} {delta}')
    blob=encode_quant(p,xy=.0625,zstep=.125);key=f'{error:g}-{name}'
    (folder/(key+'.mlq1')).write_bytes(blob)
    nx=int(round((rect[2]-rect[0])/STEP));ny=int(round((rect[3]-rect[1])/STEP))
    x,y=np.meshgrid(np.linspace(rect[0],rect[2],nx+1),np.linspace(rect[1],rect[3],ny+1))
    h=model(np.column_stack((x.ravel(),y.ravel()))).reshape(ny+1,nx+1)
    gy,gx=np.gradient(h,STEP,edge_order=2)
    np.stack((h,gx,gy),axis=2).astype('<f4').tofile(dest/(key+'.bin'))
    core=(p[:,0]>=bounds[0])&(p[:,0]<=bounds[2])&(p[:,1]>=bounds[1])&(p[:,1]<=bounds[3])
    info=dict(key=key,error=error,tile=name,bounds=bounds,fieldBounds=rect,nx=nx,ny=ny,step=STEP,
     points=len(p),corePoints=int(core.sum()),haloPoints=int((~core).sum()),bytes=len(blob),maxDifferenceFromUnsplit=float(delta))
    entries.append(info);(folder/'progress.json').write_text(json.dumps(info),encoding='utf-8');print(json.dumps(info),flush=True)
 (dest/'manifest.json').write_text(json.dumps(dict(region=region,maxWidth=2000,tiles=entries)),encoding='utf-8')
 (folder/'manifest.json').write_text(json.dumps(entries,indent=2),encoding='utf-8')

if __name__=='__main__':
 ROOT.mkdir(exist_ok=True);VIEW.mkdir(exist_ok=True)
 if len(sys.argv)>1:worker(sys.argv[1])
 else:
  def run(r):
   with (ROOT/(r+'.log')).open('w',encoding='utf-8') as log:
    p=subprocess.Popen([sys.executable,'-X','utf8',__file__,r],stdout=log,stderr=subprocess.STDOUT)
    if p.wait():raise RuntimeError(r+' の生成に失敗')
  (ROOT/'progress.json').write_text(json.dumps(dict(stage='generating')),encoding='utf-8')
  with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:list(pool.map(run,['fuji','yari','tsukuba']))
  (ROOT/'progress.json').write_text(json.dumps(dict(stage='complete')),encoding='utf-8')
