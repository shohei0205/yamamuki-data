"""元 DEM の最大誤差から LPF 幅と自然近傍の点数を決める。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json,sys
from pathlib import Path
import numpy as np
import mls_lpf_dem as lpf
from mls_dem_levels import write_json
from mls_quant_codec import encode_quant,decode_quant
from raw_dem_reference import RawDem,center_for
from sibson_surface import SibsonSurface
ROOT=Path('build/sibson-error-driven-v1')
ERROR=50.

def scan(raw,model,threshold=49.):
 n=len(model.tri);best=np.zeros(n);positions=np.zeros((n,2));maximum=0.;squared=0.;total=0
 for q,h,_ in raw.native_rows(32):
  value=model(q);err=np.abs(value-h);maximum=max(maximum,float(err.max()));squared+=float(err@err);total+=len(q)
  ids=model.triangulation.find_simplex(q);np.maximum.at(best,ids,err)
  candidate=(err>=best[ids])&(err>threshold)
  index=np.flatnonzero(candidate)
  if len(index):positions[ids[index]]=q[index]
 selected=np.flatnonzero(best>threshold);selected=selected[np.argsort(-best[selected],kind='stable')[:10000]]
 return maximum,float(np.sqrt(squared/total)),positions[selected],total

def worker(region,error=ERROR,seed_points=None,nested=False):
 if not np.isfinite(error) or error<=0:raise ValueError('許容誤差は正の有限値にしてください')
 suffix='nested' if nested else 'error'
 name=f'{error:g}-natural-{suffix}{error:g}'
 lpf.ROOT=ROOT;folder=ROOT/region/name;folder.mkdir(parents=True,exist_ok=True)
 if (folder/'info.json').exists():return
 raw=RawDem('build/terrain-cache',center_for(region));trials=[];sigma=error/2
 for attempt in range(8):
  filtered=lpf.FilteredDem(region,sigma);maximum=0.
  for q,h,_ in raw.native_rows(64):maximum=max(maximum,float(np.max(abs(filtered.sample(q)-h))))
  trials.append(dict(sigmaMeters=sigma,filterOnlyMaxError=maximum));write_json(folder/'lpf-trials.json',trials)
  lpf.progress(region,name,'filter-audit',**trials[-1])
  if maximum<=error*.5:break
  sigma*=.5
 else:raise RuntimeError('LPF の誤差予算を満たせません')
 initial=Path('build/sibson-low-count-v1')/region/'50-natural-128/model.mlq1'
 xy=decode_quant(initial.read_bytes())[:,:2].copy() if seed_points is None else np.asarray(seed_points[:,:2]).copy()
 # 外周も誤差を評価できるよう、初期状態で辺上に 1 km 間隔の点を置く。
 edge=np.linspace(-16000,16000,33)
 xy=np.unique(np.vstack((xy,np.column_stack((edge,np.full(33,-16000))),np.column_stack((edge,np.full(33,16000))),np.column_stack((np.full(33,-16000),edge)),np.column_stack((np.full(33,16000),edge)))),axis=0)
 history=[]
 for iteration in range(32):
  blob=encode_quant(np.column_stack((xy,filtered.sample(xy))),xy=.0625,zstep=.125);points=decode_quant(blob);model=SibsonSurface(points)
  maximum,rms,new,total=scan(raw,model,threshold=error*.98)
  entry=dict(iteration=iteration,controls=len(points),measuredMaxError=maximum,measuredRmse=rms,candidates=len(new));history.append(entry)
  write_json(folder/'iterations.json',history);(folder/'checkpoint.mlq1').write_bytes(blob)
  lpf.progress(region,name,'refine',**entry)
  if maximum<=error:break
  if not len(new):raise RuntimeError('誤差を超える領域の追加候補がありません')
  xy=np.unique(np.rint(np.vstack((points[:,:2],new))*16)/16,axis=0)
  if len(xy)==len(points):raise RuntimeError('量子化後に追加できる点がありません')
 else:raise RuntimeError('32 回で未達です。保存済み点を保持します')
 (folder/'model.mlq1').write_bytes(blob)
 info=dict(region=region,targetError=error,peakErrorLimit=error,n=1024,span=32000,controls=len(points),bytes=len(blob),totalBytes=len(blob),parentBytes=0,
  reconstruction='Sibson-natural-neighbor',reference='original-PNG-DEM',fittingReference='Gaussian-filtered-PNG-DEM',sigmaMeters=sigma,
  requestedSigmaMeters=error/2,filterOnlyMaxError=trials[-1]['filterOnlyMaxError'],filterErrorBudget=error*.5,
  filteredTargetKey=f'{error:g}-target-{suffix}{error:g}',lpfWidthMode='error-driven',pointCountMode='raw-DEM-max-error',
  refinementIterations=len(history),validationDomain='all-original-PNG-pixel-centers',zQuantizationMeters=.125,
  additionalDisplaySmoothing=False,filterHaloManifestSha256=filtered.digest,xyInheritedFromParent=seed_points is not None,nestedXY=nested)
 write_json(folder/'model-info.json',info);lpf.verify_export(region,error,name,raw,filtered,folder,model=model)
 dest=lpf.VIEW/region/name;tri=model.tri.astype('<u4');edges=np.unique(np.sort(np.vstack((tri[:,[0,1]],tri[:,[1,2]],tri[:,[2,0]])),axis=1),axis=0).astype('<u4')
 Path(str(dest)+'-controls.bin').write_bytes(points.astype('<f4').tobytes());Path(str(dest)+'-triangles.bin').write_bytes(tri.tobytes());Path(str(dest)+'-edges.bin').write_bytes(edges.tobytes())
 lpf.progress(region,name,'complete')

if __name__=='__main__':
 ROOT.mkdir(exist_ok=True)
 if len(sys.argv)>1:worker(sys.argv[1])
 else:
  import concurrent.futures,subprocess
  def run(r):
   with (ROOT/(r+'.log')).open('w',encoding='utf-8') as log:
    p=subprocess.Popen([sys.executable,'-X','utf8',__file__,r],stdout=log,stderr=subprocess.STDOUT);write_json(ROOT/(r+'.launch.json'),dict(pid=p.pid))
    if p.wait():raise RuntimeError(r+' の生成に失敗')
  write_json(ROOT/'progress.json',dict(stage='generating'))
  with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:list(pool.map(run,['fuji','yari','tsukuba']))
  write_json(ROOT/'progress.json',dict(stage='complete'))
