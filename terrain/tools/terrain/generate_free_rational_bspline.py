"""誤差に沿って自由配置点を追加し、既存方式の圧縮容量内で比較する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json,time,sys
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree
from scipy.stats import qmc
from raw_dem_reference import RawDem,center_for
from mls_lpf_dem import FilteredDem
from weighted_sibson import fit
from free_rational_bspline import FreeRationalSpline,supports,encode,decode
ROOT=Path('build/sibson-weighted-v1')
def write(p,v):p.write_text(json.dumps(v,ensure_ascii=False,indent=2),encoding='utf-8')
def run(region,error):
 folder=ROOT/f'{region}-{error:g}';report=json.loads((folder/'report.json').read_text());budget=report['variants']['weighted']['bytes'];filtered=FilteredDem(region,report['sigmaMeters']);raw=RawDem('build/terrain-cache',center_for(region));rng=np.random.default_rng(391)
 q=(qmc.Sobol(2,scramble=True,seed=52).random_base2(15)-.5)*16000;h=filtered.sample(q);valid=np.isfinite(h);q=q[valid];h=h[valid]
 # 初期点も格子に置かず、決定的な乱数による自由配置と外側の4点から開始。
 xy=np.vstack((rng.uniform(-8000,8000,(28,2)),[[-8500,-8500],[-8500,8500],[8500,-8500],[8500,8500]]));history=[];best=None;dense=None;started=time.perf_counter()
 held=(qmc.Sobol(2,scramble=True,seed=987).random_base2(15)-.5)*15990;truth=raw.sample(held);valid=np.isfinite(truth);held=held[valid];truth=truth[valid]
 cover_axis=np.linspace(-8000,8000,129);cx,cy=np.meshgrid(cover_axis,cover_axis);cover=np.vstack((q,np.column_stack((cx.ravel(),cy.ravel()))))
 for iteration in range(24):
  xy=np.rint(xy*16)/16;initial=filtered.sample(xy);scale,angles=supports(xy,filtered.sample,cover=cover);m=FreeRationalSpline(np.column_stack((xy,initial)),scale,angles);a=m.basis(q)
  z,w,steps=fit(a,h,initial,error,iterations=6,z_regular=.0003);m=FreeRationalSpline(np.column_stack((xy,z)),scale,angles,w);blob=encode(m);restored=decode(blob);rmse=float(np.sqrt(np.mean((restored(q)-h)**2)))
  item=dict(iteration=iteration,controls=len(xy),bytes=len(blob),trainingRmse=rmse);history.append(item);write(folder/'free-bspline-progress.json',dict(stage='fitting',budget=budget,history=history));print(region,error,item,flush=True)
  if len(xy)>=report['variants']['original']['controls']:
   dense=blob;break
  if len(blob)<=budget and (best is None or rmse<best[0]):best=(rmse,blob,xy.copy(),a,initial,scale,angles)
  residual=abs((a@(w*z))/(a@w)-h);distance,_=cKDTree(xy).query(q);order=np.argsort(-residual,kind='stable');chosen=[];cells=set();take=max(16,len(xy)//2)
  # 大きい誤差から選びつつ、直前に追加した点へ過度に集中させない。
  for ix in order:
   if distance[ix]<max(12.,16000/np.sqrt(len(xy))*.08):continue
   cell=tuple(np.floor(q[ix]/max(16.,16000/np.sqrt(len(xy))*.18)).astype(int))
   if cell in cells:continue
   cells.add(cell);chosen.append(ix)
   if len(chosen)>=take:break
  if not chosen:break
  xy=np.vstack((xy,q[chosen]))
 if best is None:raise RuntimeError('最小モデルも容量予算を超えました')
 _,blob,xy,a,initial,scale,angles=best
 # 重みなしも同じ自由配置点・同じ影響範囲で適合する。
 z,_,_=fit(a,h,initial,error,use_weights=False,z_regular=.0003);plain=encode(FreeRationalSpline(np.column_stack((xy,z)),scale,angles));versions={'free-bspline':blob,'free-bspline-unit':plain}
 if dense is not None:versions['free-bspline-dense']=dense
 axis=np.linspace(-8000,8000,513);xx,yy=np.meshgrid(axis,axis);grid=np.column_stack((xx.ravel(),yy.ravel()))
 for name,b in versions.items():
  model=decode(b)
  if name=='free-bspline-dense':
   # 高点数版は未知数に対して十分な訓練点を追加し、過適合を抑える。
   power=max(15,int(np.ceil(np.log2(len(model.points)*16))));fq=(qmc.Sobol(2,scramble=True,seed=1031).random_base2(power)-.5)*16000;fh=filtered.sample(fq);valid=np.isfinite(fh);fq=fq[valid];fh=fh[valid];fa=model.basis(fq);fz,fw,_=fit(fa,fh,filtered.sample(model.points[:,:2]),error,iterations=8,z_regular=.0003);model=FreeRationalSpline(np.column_stack((model.points[:,:2],fz)),model.scales,model.angles,fw);b=encode(model);model=decode(b)
  pred=model(held);e=pred-truth;field=model(grid).reshape(513,513);gy,gx=np.gradient(field,axis,axis);np.stack((field,gx,gy),axis=-1).astype('<f4').tofile(folder/(name+'.field'));(folder/(name+'.frb1')).write_bytes(b)
  report['variants'][name]=dict(controls=len(model.points),bytes=len(b),rawRmse=float(np.sqrt(np.mean(e*e))),rawMaxError=float(abs(e).max()),sampleCount=len(e),weightMinimum=float(model.weights.min()),weightMaximum=float(model.weights.max()),geometry='free-oriented-cubic-B-spline-bases',standardNurbs=False)
  controls=np.column_stack((model.points,model.weights,model.scales,model.angles,model(model.points[:,:2])));(folder/(name+'.controls.json')).write_text(json.dumps(controls.tolist()),encoding='utf-8')
 report['freeSpline']=dict(budget=budget,history=history,seconds=time.perf_counter()-started,selection='train-residual-descending-with-distance-thinning',testDomain='16km square',formula='sum(B3(u_i)*B3(v_i)*w_i*z_i)/sum(B3(u_i)*B3(v_i)*w_i)',axes='per-point rotation and local scale; no control rows/columns',standardNurbs=False)
 write(folder/'report.json',report);write(folder/'free-bspline-progress.json',dict(stage='complete',history=history));print('complete',region,error,report['variants']['free-bspline'],flush=True)
if __name__=='__main__':
 for region in (sys.argv[1:] or ['fuji','tsukuba','yari']):
  for error in [50.,25.]:run(region,error)
