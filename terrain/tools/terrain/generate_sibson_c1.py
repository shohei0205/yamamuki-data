"""自然近傍の曲面変更と表示近似を分離して3地域で比較する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json,time
from pathlib import Path
import numpy as np
from scipy.stats import qmc
from scipy.ndimage import map_coordinates
from raw_dem_reference import RawDem,center_for
from mls_quant_codec import decode_quant
from sibson_aux_surface import AuxiliarySibsonSurface
from weighted_sibson import WeightedSibson
from sibson_c1 import SibsonC1
ROOT=Path('build/sibson-weighted-v1')
def write(p,v):
 temp=p.with_suffix('.tmp');temp.write_text(json.dumps(v,ensure_ascii=False,indent=2),encoding='utf-8');temp.replace(p)
def run(region,error):
 folder=ROOT/f'{region}-{error}';report=json.loads((folder/'report.json').read_text());points=decode_quant((folder/'original.mlq1').read_bytes());raw=RawDem('build/terrain-cache',center_for(region));start=time.perf_counter();basis=WeightedSibson(points);full=SibsonC1(points,basis_model=basis);limited=SibsonC1(points,gradient=full.gradient,limited=True,basis_model=basis);native=AuxiliarySibsonSurface(points);build_seconds=time.perf_counter()-start
 def both(q):
  output=np.empty((len(q),2))
  for first in range(0,len(q),16384):
   sub=q[first:first+16384];a=basis.basis(sub);output[first:first+len(sub),0]=full.from_basis(sub,a);output[first:first+len(sub),1]=limited.from_basis(sub,a)
  return output
 held=(qmc.Sobol(2,scramble=True,seed=987).random_base2(15)-.5)*15990;truth=raw.sample(held);valid=np.isfinite(truth);held=held[valid];truth=truth[valid];pred=both(held);baseline=native(held);axis=np.linspace(-8000,8000,513);x,y=np.meshgrid(axis,axis);q=np.column_stack((x.ravel(),y.ravel()));write(ROOT/'sibson-c1-progress.json',dict(region=region,error=error,stage='evaluating-c1'));height=both(q);gradient=[]
 for delta in [np.array([1.,0]),np.array([0,1.])]:gradient.append((both(q+delta)-both(q-delta))/2)
 for i,name in enumerate(['sibson-c1','sibson-c1-limited']):
  np.column_stack((height[:,i],gradient[0][:,i],gradient[1][:,i])).astype('<f4').tofile(folder/(name+'.field'));residual=pred[:,i]-truth;report['variants'][name]=dict(controls=len(points),bytes=report['variants']['original']['bytes'],rawRmse=float(np.sqrt(np.mean(residual*residual))),rawMaxError=float(abs(residual).max()),sampleCount=len(held),geometry='Sibson-C1',gradientSource='Sibson-natural-coordinate-gradient-fit-to-stored-XYZ',extraDistributedBytes=0,gradientRamBytes=full.gradient.nbytes,gradientBuildSeconds=build_seconds,limiting=name.endswith('limited'),limitedPoints=int(np.sum(limited.limiting<1)),fieldResolution=512)
 # 補間式を変えず、表示の微分・評価密度だけを精密化する。
 for n,name in [(512,'natural-precise-shade'),(1024,'natural-fine')]:
  write(ROOT/'sibson-c1-progress.json',dict(region=region,error=error,stage='evaluating-original',n=n));a=np.linspace(-8000,8000,n+1);x,y=np.meshgrid(a,a);q=np.column_stack((x.ravel(),y.ravel()));h=native(q);grad=[]
  for delta in [np.array([1.,0]),np.array([0,1.])]:grad.append((native(q+delta)-native(q-delta))/2)
  field=np.column_stack((h,*grad)).astype('<f4');field.tofile(folder/(name+'.field'));uv=(held+8000)/(16000/n);view=map_coordinates(field[:,0].reshape(n+1,n+1),[uv[:,1],uv[:,0]],order=1,prefilter=False);display_difference=view-baseline;residual=baseline-truth;report['variants'][name]=dict(controls=len(points),bytes=report['variants']['original']['bytes'],rawRmse=float(np.sqrt(np.mean(residual*residual))),rawMaxError=float(abs(residual).max()),sampleCount=len(held),geometry='original-Sibson',extraDistributedBytes=0,fieldResolution=n,displayApproximationRmse=float(np.sqrt(np.mean(display_difference*display_difference))),displayApproximationMaximum=float(abs(display_difference).max()),displayNormal='central-difference-1m-on-original-curve')
  if n==512:
   old=np.fromfile(folder/'original.field','<f4').reshape(-1,3);difference=float(abs(old[:,0]-field[:,0]).max())
   if difference>1e-4:raise ValueError('精密陰影版が元の曲面を変更しています')
   report['variants'][name]['maximumStoredHeightDifference']=difference
 report['sibsonC1']=dict(seconds=time.perf_counter()-start,algorithmReference='https://doc.cgal.org/latest/Interpolation/index.html#InterpolationSibson',source='original control XYZ, no DEM gradients persisted',comparisons=['sibson-c1','sibson-c1-limited','natural-precise-shade','natural-fine']);write(folder/'report.json',report);print(region,error,{k:{a:report['variants'][k][a] for a in ['rawRmse','rawMaxError']} for k in report['sibsonC1']['comparisons']},flush=True)
if __name__=='__main__':
 for region in ['fuji','tsukuba','yari']:
  for error in [25,50]:run(region,error)
 write(ROOT/'sibson-c1-progress.json',dict(stage='complete'))
