"""制御点の局所的な丸めだけを自然近傍の精密表示と比較する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json,time
from pathlib import Path
import numpy as np
from scipy.stats import qmc
from raw_dem_reference import RawDem,center_for
from mls_quant_codec import decode_quant
from sibson_aux_surface import AuxiliarySibsonSurface
from sibson_node_rounding import RoundedSibson
ROOT=Path('build/sibson-weighted-v1')
def write(p,v):
 tmp=p.with_suffix('.tmp');tmp.write_text(json.dumps(v,ensure_ascii=False,indent=2),encoding='utf-8');tmp.replace(p)
def run(region,error):
 folder=ROOT/f'{region}-{error}';report=json.loads((folder/'report.json').read_text());p=decode_quant((folder/'original.mlq1').read_bytes());(folder/'original.controls.json').write_text(json.dumps(np.column_stack((p,np.ones(len(p)),np.zeros((len(p),3)),p[:,2])).tolist()),encoding='utf-8');base=AuxiliarySibsonSurface(p);names=['natural-rounded-small','natural-rounded'];models=[RoundedSibson(p,f,base=base) for f in [.2,.45]];raw=RawDem('build/terrain-cache',center_for(region));held=(qmc.Sobol(2,scramble=True,seed=987).random_base2(15)-.5)*15990;truth=raw.sample(held);valid=np.isfinite(truth);held=held[valid];truth=truth[valid];baseline=base(held);predictions=[m.from_base(held,baseline) for m in models];axis=np.linspace(-8000,8000,1025);x,y=np.meshgrid(axis,axis);q=np.column_stack((x.ravel(),y.ravel()));start=time.perf_counter();write(ROOT/'node-rounding-progress.json',dict(region=region,error=error,stage='evaluating-display'));baseline=base(q);heights=[m.from_base(q,baseline) for m in models];gradients=[[],[]]
 for delta in [np.array([1.,0]),np.array([0,1.])]:
  a=base(q+delta);b=base(q-delta)
  for i,m in enumerate(models):gradients[i].append((m.from_base(q+delta,a)-m.from_base(q-delta,b))/2)
 for i,(name,m) in enumerate(zip(names,models)):
  field=np.column_stack((heights[i],*gradients[i])).astype('<f4');field.tofile(folder/(name+'.field'));e=predictions[i]-truth;distance,ids=m.tree.query(held);active=(m.radius[ids]>0)&(distance<m.radius[ids]);unchanged=~active
  if not np.array_equal(predictions[i][unchanged],base(held[unchanged])):raise ValueError('丸め範囲外の曲面が変化しました')
  difference=heights[i]-baseline;report['variants'][name]=dict(controls=len(p),bytes=report['variants']['original']['bytes'],extraDistributedBytes=0,rawRmse=float(np.sqrt(np.mean(e*e))),rawMaxError=float(abs(e).max()),sampleCount=len(e),geometry='Sibson-with-local-node-rounding',fieldResolution=1024,radiusFraction=m.fraction,minimumRadius=float(m.radius[m.radius>0].min()),maximumRadius=float(m.radius.max()),affectedSampleFraction=float(active.mean()),maximumHeightChange=float(abs(difference).max()),roundingBuildSeconds=m.build_seconds,extraCacheBytes=m.radius.nbytes+m.coefficients.nbytes,controlPointMaximumError=float(abs(m(p[:,:2])-p[:,2]).max()),outsideSupportUnchanged=True)
 report['nodeRounding']=dict(seconds=time.perf_counter()-start,source='original Sibson curve, not DEM samples',formula='original + (1-10t^3+15t^4-6t^5)*(local-quadratic-original)',fitting='two rings at 0.5R and R, fixed center Z, 32 angles each',overlap='R < 0.5 nearest-neighbor distance, no overlapping node patches');write(folder/'report.json',report);print(region,error,[report['variants'][name] for name in names],flush=True)
if __name__=='__main__':
 for region in ['fuji','tsukuba','yari']:
  for error in [25,50]:run(region,error)
 write(ROOT/'node-rounding-progress.json',dict(stage='complete'))
