"""円形補正を使わず、自然近傍全体に適合したC1曲面を比較する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json,time,sys
from pathlib import Path
import numpy as np
from scipy.stats import qmc
from raw_dem_reference import RawDem,center_for
from mls_quant_codec import decode_quant
from sibson_aux_surface import AuxiliarySibsonSurface
from sibson_global_c1 import GlobalC1
ROOT=Path('build/sibson-weighted-v1')
def write(p,v):
 tmp=p.with_suffix('.tmp');tmp.write_text(json.dumps(v,ensure_ascii=False,indent=2),encoding='utf-8');tmp.replace(p)
def run(region,error):
 folder=ROOT/f'{region}-{error}';report=json.loads((folder/'report.json').read_text());points=decode_quant((folder/'original.mlq1').read_bytes());base=AuxiliarySibsonSurface(points);raw=RawDem('build/terrain-cache',center_for(region));held=(qmc.Sobol(2,scramble=True,seed=987).random_base2(15)-.5)*15990;truth=raw.sample(held);valid=np.isfinite(truth);held=held[valid];truth=truth[valid];original=base(held);axis=np.linspace(-8000,8000,1025);x,y=np.meshgrid(axis,axis);grid=np.column_stack((x.ravel(),y.ravel()))
 for strength,name in [(.001,'natural-global-c1-soft'),(.01,'natural-global-c1')]:
  started=time.perf_counter();status=dict(region=region,error=error,variant=name,strength=strength,stage='fitting');write(ROOT/'global-c1-progress.json',status)
  def progress(item):write(ROOT/'global-c1-progress.json',dict(**{**status,**item},elapsedSeconds=time.perf_counter()-started))
  model=GlobalC1(points,strength=strength,progress=progress);prediction=model(held);e=prediction-truth;d=prediction-original;write(ROOT/'global-c1-progress.json',dict(**{**status,'stage':'evaluating-display'},fit=model.fit_stats));h=model(grid);grad=[]
  for delta in [np.array([1.,0]),np.array([0,1.])]:grad.append((model(grid+delta)-model(grid-delta))/2)
  np.column_stack((h,*grad)).astype('<f4').tofile(folder/(name+'.field'));np.savez_compressed(folder/(name+'.cache.npz'),points=points,gradient=model.gradient);item=dict(controls=len(points),bytes=report['variants']['original']['bytes'],extraDistributedBytes=0,rawRmse=float(np.sqrt(np.mean(e*e))),rawMaxError=float(abs(e).max()),sampleCount=len(e),geometry='Sibson-C1-with-global-gradient-fit',fieldResolution=1024,fit=model.fit_stats,sourceCurveRmse=float(np.sqrt(np.mean(d*d))),sourceCurveMaximumDifference=float(abs(d).max()),controlPointMaximumError=float(abs(model(points[:,:2])-points[:,2]).max()),noCircularPatches=True,seconds=time.perf_counter()-started)
  report['variants'][name]=item;write(folder/'report.json',report);print(region,error,name,item,flush=True)
if __name__=='__main__':
 for region in (sys.argv[1:] or ['fuji','tsukuba','yari']):
  for error in [25,50]:run(region,error)
 write(ROOT/'global-c1-progress.json',dict(stage='complete'))
