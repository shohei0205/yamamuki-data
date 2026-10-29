"""元DEM全有効画素のSibson標高誤差を再評価して追加する比較生成器。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json,time
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree
from scipy.stats import qmc
from natural_c1_128 import ROOT,RUN as BASE,write,GlobalC1,close_cells,pack
from continuous_dem import pixels,STEP
from sibson_aux_surface import AuxiliarySibsonSurface
OUT=ROOT/'build/natural-raw-c1-128-errorfit-v2'

def build(region,x,y,error=12.5):
 start=time.monotonic();src=BASE/region/f'{error:g}'/f'{x}-{y}';dst=OUT/region/f'{error:g}'/f'{x}-{y}';dst.mkdir(parents=True,exist_ok=True)
 if (dst/'selection.json').exists():return
 info=json.loads((src/'packet.json').read_text());world=np.asarray(json.loads((BASE/region/'source/region.json').read_text())['origin'])+np.asarray(info['origin']);side=info['side'];halo=info['halo'];half=side/2+halo
 audit_half=side/2+250
 region_origin=np.asarray(json.loads((BASE/region/'source/region.json').read_text())['origin']);clip_lo=region_origin+60000-250;clip_hi=region_origin+68000+250;world_lo=np.maximum(world-audit_half,clip_lo);world_hi=np.minimum(world+audit_half,clip_hi)
 lo=np.ceil(world_lo/STEP-.5).astype(int);hi=np.floor(world_hi/STEP-.5).astype(int);xx,yy=np.meshgrid(np.arange(lo[0],hi[0]+1),np.arange(lo[1],hi[1]+1));heights=pixels(xx,yy);valid=np.isfinite(heights);q=np.column_stack(((xx[valid]+.5)*STEP-world[0],(yy[valid]+.5)*STEP-world[1]));target=heights[valid];core=np.all(abs(q)<=side/2,axis=1)
 original=np.load(src/'control-gradient.npz')['points'];initial=ROOT/f'build/natural-raw-c1-128-quarter-v1/{region}/{error:g}/{x}-{y}/control-gradient.npz'
 if (dst/'fit-state.npz').exists():p=np.load(dst/'fit-state.npz')['points'].copy()
 elif initial.exists():p=np.load(initial)['points'].copy()
 else:
  real=np.all(abs(original[:,:2])<=half+1,axis=1);p=np.vstack((original[~real],original[real][::4]))
 initial_count=len(p)-4;history=[]
 for iteration in range(160):
  model=AuxiliarySibsonSurface(p);predicted=model(q);residual=abs(predicted-target);maximum=float(residual.max()) if len(residual) else 0;core_max=float(residual[core].max()) if core.any() else 0
  history.append(dict(iteration=iteration,points=len(p)-4,max=maximum,coreMax=core_max));write(dst/'progress.json',dict(stage='evaluate-all-dem-pixels',iteration=iteration,points=len(p)-4,pixels=len(q),max=maximum,coreMax=core_max,seconds=time.monotonic()-start))
  if maximum<=error:break
  # 優先度は誤差。バッチ内だけ離して選び、次のバッチ前に全域を再評価する。
  order=np.argsort(residual)[::-1];known=set(map(tuple,np.rint((p[:,:2]+world)/.0625).astype(np.int64)));chosen=[];added=[]
  for i in order:
   if residual[i]<=error:break
   quant=np.rint((q[i]+world)/.0625).astype(np.int64);key=tuple(quant)
   if key in known or any(np.linalg.norm(q[i]-a)<64 for a in chosen):continue
   chosen.append(q[i]);added.append([*(quant*.0625-world),round(float(target[i])/.125)*.125]);known.add(key)
   if len(added)==64:break
  if not added:raise ValueError('誤差超過だが未登録の追加点が無い')
  p=np.vstack((p,added));np.savez_compressed(dst/'fit-state.npz',points=p)
  if len(p)>20000:raise ValueError('比較用安全上限20000点を超えたため未完了')
 else:raise ValueError('反復上限に到達、未完了')
 bounds=np.asarray(info['bounds']);n=max(16384,min(262144,2**int(np.ceil(np.log2(len(p)*4)))));queries=qmc.Sobol(2,scramble=True,seed=9124).random_base2(int(np.log2(n)))*(bounds[2:]-bounds[:2])+bounds[:2];c1=GlobalC1(p,strength=.01,queries=queries);close_cells(c1.fast_model)
 meta={k:info[k] for k in ['region','error','x','y','side','origin','coreBounds','worldFieldBounds','halo']};meta.update(curve='Sibson-error-driven-dem-pixels',lpf=False,sourceGeometryBytes=len(p)*12,maxErrorGuaranteed=False,allDemPixelsAudited=(error==12.5),regionalDemPixelsAudited=True,auditWorldBounds=[*world_lo,*world_hi],auditedErrorTarget=error,auditedMaxError=maximum,auditedCoreMaxError=core_max,auditScope='usual-Sibson at original valid DEM pixel centers, within mountain-centered 8km comparison window plus 250m band, with outer support points retained; excludes outside window, C1, rendered grid and tile blending')
 packet,out=pack(p,c1.gradient,bounds,meta,model=c1.fast_model,max_candidates=48,max_depth=11)
 if out['unsafeLeaves']:raise ValueError('unsafe GPU Voronoi leaves')
 audit=dict(errorTarget=error,auditWorldBounds=[*world_lo,*world_hi],regionalWindowMeters=8000,validPixels=len(q),corePixels=int(core.sum()),max=maximum,coreMax=core_max,rmse=float(np.sqrt(np.mean(residual*residual))),initialPoints=initial_count,finalPoints=len(p)-4,originalPoints=len(original)-4,minSpacingMeters=float(cKDTree(p[np.all(abs(p[:,:2])<=half+1,axis=1),:2]).query(p[np.all(abs(p[:,:2])<=half+1,axis=1),:2],k=2)[0][:,1].min()),history=history,seconds=time.monotonic()-start)
 (dst/'packet.bin').write_bytes(packet);np.savez_compressed(dst/'control-gradient.npz',points=p,gradient=c1.gradient);out['errorFitAudit']=audit;write(dst/'packet.json',out);write(dst/'selection.json',audit);print(region,x,y,'ready',len(p)-4,maximum,flush=True)

def run():
 OUT.mkdir(exist_ok=True);done=[];errors={}
 for region in ['fuji','yari','tsukuba']:
  for x,y in [(31,31),(32,31),(31,32),(32,32)]+[(x,y) for y in range(30,34) for x in range(30,34) if (x,y) not in [(31,31),(32,31),(31,32),(32,32)]]:
   try:build(region,x,y);done.append([region,x,y])
   except Exception as e:errors[f'{region}/{x}-{y}']=str(e);print('FAILED',region,x,y,str(e),flush=True)
   write(OUT/'progress.json',dict(done=len(done),total=48,complete=len(done)==48,failed=errors))
if __name__=='__main__':run()
