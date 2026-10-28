"""標準NURBSの制御点数を約2倍・4倍に増やして再適合する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json,time,sys
from pathlib import Path
import numpy as np
from scipy.stats import qmc
from raw_dem_reference import RawDem,center_for
from mls_lpf_dem import FilteredDem
from standard_nurbs import refine_to,fit_nurbs,encode,decode
ROOT=Path('build/sibson-weighted-v1')
def write(p,v):
 tmp=p.with_suffix('.tmp');tmp.write_text(json.dumps(v,ensure_ascii=False,indent=2),encoding='utf-8');tmp.replace(p)
def run(region,error):
 folder=ROOT/f'{region}-{error:g}';report=json.loads((folder/'report.json').read_text());base=decode((folder/'standard-nurbs.nrb1').read_bytes());base_n=len(base.tu)-4;raw=RawDem('build/terrain-cache',center_for(region));filtered=FilteredDem(region,report['sigmaMeters']);held=(qmc.Sobol(2,scramble=True,seed=987).random_base2(15)-.5)*15990;truth=raw.sample(held);valid=np.isfinite(truth);q=held[valid];truth=truth[valid]
 for factor in [2,4]:
  name=f'standard-nurbs-{factor}x'
  if name in report['variants'] and (folder/(name+'.field')).exists():continue
  start=time.perf_counter();n=int(np.ceil(base_n*np.sqrt(factor)));model=refine_to(base,n);uv=qmc.Sobol(2,scramble=True,seed=188).random_base2(12);difference=float(abs(model.evaluate(uv)-base.evaluate(uv)).max())
  if difference>1e-7:raise ValueError('節点挿入が元曲面を変更しました')
  status=dict(region=region,error=error,variant=name,n=n,controls=n*n,baseControls=len(base.points),factor=factor,stage='fitting')
  def progress(item):write(ROOT/'standard-larger-progress.json',dict(**status,iteration=item['iteration'],objective=item['objective'],elapsedSeconds=time.perf_counter()-start))
  progress(dict(iteration=-1,objective=None));model,history=fit_nurbs(model.points[:,:2],n,filtered.sample,error,initial=model,iterations=6,progress=progress);blob=encode(model);(folder/(name+'.checkpoint.nrb1')).write_bytes(blob);model=decode(blob)
  position,su,sv=model.evaluate(uv,True);det=su[:,0]*sv[:,1]-su[:,1]*sv[:,0]
  if np.any(det<=0):raise ValueError('保存後の曲面のXY写像が反転しています')
  write(ROOT/'standard-larger-progress.json',dict(**{**status,'stage':'evaluating'},elapsedSeconds=time.perf_counter()-start));prediction=model(q);e=prediction-truth;uvcheck=model.inverse_xy(q);inverse_error=float(abs(model.evaluate(uvcheck)[:,:2]-q).max());axis=np.linspace(-8000,8000,513);x,y=np.meshgrid(axis,axis);grid=np.column_stack((x.ravel(),y.ravel()));model.height_gradient(grid).astype('<f4').tofile(folder/(name+'.field'));(folder/(name+'.nrb1')).write_bytes(blob)
  controls=np.column_stack((model.points,model.weights,np.zeros((len(model.points),3)),model(model.points[:,:2])));(folder/(name+'.controls.json')).write_text(json.dumps(controls.tolist()),encoding='utf-8')
  item=dict(controls=n*n,controlNet=[n,n],bytes=len(blob),rawRmse=float(np.sqrt(np.mean(e*e))),rawMaxError=float(abs(e).max()),sampleCount=len(e),weightMinimum=float(model.weights.min()),weightMaximum=float(model.weights.max()),geometry='standard-bicubic-NURBS',standardNurbs=True,adaptiveXY=True,minimumSampledJacobian=float(det.min()),inverseMaximumError=inverse_error,training=history,requestedControlMultiplier=factor,actualControlMultiplier=n*n/len(base.points),parent='standard-nurbs',initialKnotInsertionMaximumDifference=difference,seconds=time.perf_counter()-start)
  report['variants'][name]=item;write(folder/'report.json',report);write(ROOT/'standard-larger-progress.json',dict(**{**status,'stage':'complete-case'},seconds=time.perf_counter()-start));print(region,error,name,{k:item[k] for k in ['controls','bytes','rawRmse','rawMaxError','seconds']},flush=True)
if __name__=='__main__':
 for region in (sys.argv[1:] or ['fuji','tsukuba','yari']):
  for error in [25.,50.]:run(region,error)
 write(ROOT/'standard-larger-progress.json',dict(stage='complete'))
