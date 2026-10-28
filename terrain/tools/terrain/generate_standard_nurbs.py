"""標準NURBSの等間隔・地形適応制御網を、保存後の曲面で比較する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json,time,sys
from pathlib import Path
import numpy as np
from scipy.stats import qmc
from raw_dem_reference import RawDem,center_for
from mls_lpf_dem import FilteredDem
from standard_nurbs import NurbsSurface,regular_xy,adapted_xy,knots,fit_nurbs,encode,decode
ROOT=Path('build/sibson-weighted-v1')
def write(p,v):p.write_text(json.dumps(v,ensure_ascii=False,indent=2),encoding='utf-8')
def run(region,error):
 folder=ROOT/f'{region}-{error:g}';report=json.loads((folder/'report.json').read_text());raw=RawDem('build/terrain-cache',center_for(region));filtered=FilteredDem(region,report['sigmaMeters']);n=int(np.ceil(np.sqrt(report['variants']['original']['controls'])));regular=regular_xy(n);adapted=adapted_xy(n,filtered.sample);uv=qmc.Sobol(2,scramble=True,seed=1893).random_base2(14);t=knots(n);start=time.perf_counter()
 for name,xy,weighted in [('standard-nurbs-uniform',regular,False),('standard-nurbs-unit',adapted,False),('standard-nurbs',adapted,True)]:
  write(folder/'standard-nurbs-progress.json',dict(stage='fitting',region=region,error=error,variant=name,n=n,controls=n*n));model,history=fit_nurbs(xy,n,filtered.sample,error,weighted=weighted,iterations=6);blob=encode(model);(folder/(name+'.checkpoint.nrb1')).write_bytes(blob);model=decode(blob);_,u,v=model.evaluate(uv,True);det=u[:,0]*v[:,1]-u[:,1]*v[:,0]
  if np.any(det<=0):raise ValueError('保存後のNURBSが折り返しています')
  held=(qmc.Sobol(2,scramble=True,seed=987).random_base2(15)-.5)*15990;truth=raw.sample(held);valid=np.isfinite(truth);q=held[valid];pred=model(q);e=pred-truth[valid];qcheck=model.evaluate(model.inverse_xy(q))[:,:2];xy_error=float(abs(qcheck-q).max())
  axis=np.linspace(-8000,8000,513);xx,yy=np.meshgrid(axis,axis);grid=np.column_stack((xx.ravel(),yy.ravel()));write(folder/'standard-nurbs-progress.json',dict(stage='evaluating-display',region=region,error=error,variant=name,n=n));field=model.height_gradient(grid).astype('<f4');field.tofile(folder/(name+'.field'));(folder/(name+'.nrb1')).write_bytes(blob)
  points=model.points;surface_height=model(points[:,:2]);controls=np.column_stack((points,model.weights,np.zeros((len(points),3)),surface_height));(folder/(name+'.controls.json')).write_text(json.dumps(controls.tolist()),encoding='utf-8')
  item=dict(controls=n*n,controlNet=[n,n],bytes=len(blob),rawRmse=float(np.sqrt(np.mean(e*e))),rawMaxError=float(abs(e).max()),sampleCount=len(e),weightMinimum=float(model.weights.min()),weightMaximum=float(model.weights.max()),geometry='standard-bicubic-NURBS',standardNurbs=True,adaptiveXY=name!='standard-nurbs-uniform',minimumSampledJacobian=float(det.min()),inverseMaximumError=xy_error,training=history,xyDisplacementMaximum=float(np.linalg.norm(xy-regular,axis=1).max()),knots=model.tu.tolist())
  report['variants'][name]=item;write(folder/'report.json',report);print(region,error,name,{k:item[k] for k in ['controls','bytes','rawRmse','rawMaxError','inverseMaximumError','xyDisplacementMaximum']},flush=True)
 report['standardNurbs']=dict(degrees=[3,3],formula='S(u,v)=sum(N_i,3(u)*M_j,3(v)*w_ij*P_ij)/sum(N_i,3(u)*M_j,3(v)*w_ij)',controlNet='rectangular topology, terrain-adapted physical XY',seconds=time.perf_counter()-start,display='invert physical XY and evaluate analytical surface normal',sourceReference='original PNG DEM with existing LPF',quantization=dict(xy=.0625,z=.125,weightLog2Step=1/32));write(folder/'report.json',report);write(folder/'standard-nurbs-progress.json',dict(stage='complete',region=region,error=error,n=n))
if __name__=='__main__':
 for region in (sys.argv[1:] or ['fuji','tsukuba','yari']):
  for error in [50.,25.]:run(region,error)
