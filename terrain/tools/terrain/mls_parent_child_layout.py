"""子の再現誤差を親点の再配置へ戻す同一点数の比較。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import json,sqlite3,zlib,time
from pathlib import Path
import numpy as np
from scipy.ndimage import gaussian_filter
from scipy.sparse.linalg import lsmr
from scipy.stats import qmc
from free_mls import operator,LocalSupport,reference_values,SPAN
from mls_generated_stable import generate_linear,DirectSupport,encode,decode
from mls_error_quality import peak_samples,requirements
from adaptive_allocation_surface import distribute
from mls_peak_allocation import peak_density

def pin_corners(xy):
 p=np.rint(xy*4)/4;used=set()
 for corner in ((-16000,-16000),(-16000,16000),(16000,-16000),(16000,16000)):
  order=np.argsort(np.linalg.norm(p-corner,axis=1));i=next(int(v) for v in order if int(v) not in used);p[i]=corner;used.add(i)
 if len(np.unique(p,axis=0))!=len(p):raise ValueError('量子化後の位置が重複')
 return p.astype('<f4')
def fit(a,truth,initial,limit,iterations,fixed=0):
 z=initial.copy();weights=1/limit;best=None
 for it in range(iterations):
  dz=lsmr(a.multiply(weights[:,None]),(truth-fixed-a@z)*weights,damp=.003,maxiter=500,atol=1e-7,btol=1e-7)[0];z+=dz;err=fixed+a@z-truth;score=float(np.max(abs(err)/limit))
  if best is None or score<best[0]:best=(score,z.copy())
  if score<.8:break
  weights=np.maximum(weights,np.clip(abs(err)/limit,1,24)**2/limit)
 return best[1]
def main():
 out=Path('build/mls-parent-child-layout-v1');out.mkdir(exist_ok=True)
 with sqlite3.connect('build/free-mls-v3/mls.sqlite3') as db:b=db.execute("SELECT points FROM surfaces WHERE region='yari' AND budget=8192").fetchone()[0]
 old=np.frombuffer(zlib.decompress(b),'<f4')[1:].reshape(-1,3);count=len(old)
 with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:b=db.execute("SELECT coefficients FROM surfaces WHERE region='yari' AND n=256").fetchone()[0]
 original=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261)
 axis=np.linspace(-16000,16000,257);x,y=np.meshgrid(axis,axis);grid=np.column_stack((x.ravel(),y.ravel()));base_train=np.vstack((grid,(qmc.Sobol(2,scramble=True,seed=17004).random_base2(14)-.5)*SPAN,peak_samples(13,17452,1500)));base_truth=reference_values(original,base_train)
 valid=np.vstack(((qmc.Sobol(2,scramble=True,seed=17904).random_base2(15)-.5)*SPAN,peak_samples(13,17935,1000)));vt=reference_values(original,valid)
 density=peak_density(original,.1,count=count);density/=density.mean();reports=[]
 for iteration in range(3):
  started=time.perf_counter();folder=out/f'iteration-{iteration}';folder.mkdir(exist_ok=True)
  xy=pin_corners(old[:,:2] if iteration==0 else distribute(density,count,SPAN/np.sqrt(count)))
  initial=reference_values(original,xy);parent_train=np.vstack((base_train,xy));pt=np.concatenate((base_truth,initial));ps=LocalSupport(xy,pre_smooth=False,floor_factor=.5);ap=operator(xy,parent_train,ps,curvature=1e-5);parent_limit=np.where(np.linalg.norm(parent_train,axis=1)<=2000,25.,50.)
  zp=fit(ap,pt,initial,parent_limit,3);parent=np.column_stack((xy,zp)).astype('<f4');parent.tofile(folder/'parent.f32')
  added=generate_linear(parent,1);allxy=np.vstack((xy,added[:,:2]));train=np.vstack((base_train,allxy));truth=np.concatenate((base_truth,reference_values(original,allxy)));support=DirectSupport(allxy);a=operator(allxy,train,support,curvature=1e-5);n=len(parent);fixed=a[:,:n]@parent[:,2];initial=reference_values(original,added[:,:2]);limits=np.where(np.linalg.norm(train,axis=1)<=2000,12.5,25.)
  z=fit(a[:,n:],truth,initial,limits,6,fixed);np.save(folder/'child-z.npy',z);ag=operator(allxy,grid,support,curvature=1e-5);av=operator(allxy,valid,support,curvature=1e-5);raw_full=np.concatenate((parent[:,2],z));raw_error=av@raw_full-vt;error_grid=(ag@raw_full-reference_values(original,grid)).reshape(257,257)
  best=None;trials=[]
  for step in (1,4,8,16):
   blob=encode(parent,z,1,step);p=np.vstack((parent,decode(parent,blob)));measurement=requirements(av@p[:,2]-vt,valid,25);audit=requirements(a@p[:,2]-truth,train,25);score=max(measurement['measuredMaxError']/25,measurement['measuredPeakMaxError']/12.5);item=dict(step=step,bytes=len(blob),audit=audit,validation=measurement,score=score);trials.append(item)
   key=(not (measurement['converged'] and audit['converged']),len(blob) if measurement['converged'] and audit['converged'] else score)
   if best is None or key<best[0]:best=(key,item,blob)
  _,chosen,blob=best;(folder/'residual.mlg2').write_bytes(blob);report=dict(iteration=iteration,parentControls=len(parent),added=len(added),controls=len(allxy),rawValidation=requirements(raw_error,valid,25),**chosen,seconds=time.perf_counter()-started);reports.append(report);(folder/'info.json').write_text(json.dumps(report,indent=2),encoding='utf-8');print(json.dumps(report),flush=True);(out/'report.json').write_text(json.dumps(reports,indent=2),encoding='utf-8')
  # 子の大きな残差を滑らかな二次元密度へ戻す。全追加点採用の規則は変更しない。
  scale=np.where(np.hypot(x,y)<=2000,12.5,25);feedback=np.sqrt(gaussian_filter((error_grid/scale)**2,2));feedback=np.maximum(feedback,.02);feedback/=feedback.mean();importance=density*np.clip(np.sqrt(feedback),.5,2.5);importance/=importance.mean();density=.35*peak_density(original,.1,count=count)+.65*importance;density=np.clip(density,.15,12);density/=density.mean()
 # 比較用点で選び、さらに未使用の点で確認する。
 selected=min(reports,key=lambda r:(not(r['validation']['converged'] and r['audit']['converged']),r['bytes'] if r['validation']['converged'] and r['audit']['converged'] else r['score']));folder=out/f"iteration-{selected['iteration']}";parent=np.fromfile(folder/'parent.f32','<f4').reshape(-1,3);p=np.vstack((parent,decode(parent,(folder/'residual.mlg2').read_bytes())))
 finalq=np.vstack(((qmc.Sobol(2,scramble=True,seed=18904).random_base2(15)-.5)*SPAN,peak_samples(13,18935,1000)));error=operator(p[:,:2],finalq,DirectSupport(p[:,:2]),curvature=1e-5)@p[:,2]-reference_values(original,finalq)
 result=dict(selectedIteration=selected['iteration'],finalValidation=requirements(error,finalq,25),reports=reports);(out/'selected.json').write_text(json.dumps(result,indent=2),encoding='utf-8');print('FINAL',json.dumps(result),flush=True)
if __name__=='__main__':main()
