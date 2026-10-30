"""精細な元曲面から、元DEM誤差を守ってSibson制御点を削除する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import heapq,json,time,traceback,sqlite3
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path
import numpy as np
from scipy.stats import qmc
import errorfit_full_tile as fit
from sibson_aux_surface import AuxiliarySibsonSurface
from natural_c1_128 import GlobalC1,close_cells,pack,write,ROOT

from continuous_dem import tile,STEP
OUT=ROOT/'build/natural-error-pruned-v1'

def reduce_points(points,q,z,error,protected,progress=lambda **v:None,active=None):
 """削除点での残差を優先度の見込みとし、採否は全評価点で判定する。"""
 active=np.ones(len(points),bool) if active is None else active.copy();serial=np.zeros(len(points),np.int64);heap=[];attempted=0;removed=int(len(points)-active.sum());started=time.monotonic()
 def surface(mask):return AuxiliarySibsonSurface(points[mask])
 model=surface(active);residual=abs(model(q)-z)
 if residual.max()>error:raise ValueError(f'元曲面が削減誤差を満たしていません: {residual.max()} > {error}')
 def queue(ids):
  for k in sorted(set(map(int,ids))):
   serial[k]+=1
   if not active[k] or protected[k]:continue
   mask=active.copy();mask[k]=False;trial=surface(mask)
   # 正確な全域誤差を低コストに順位付けできるわけではない。これは見込み。
   cost=abs(float(trial(points[k:k+1,:2])[0])-points[k,2])
   heapq.heappush(heap,(cost,k,int(serial[k])))
 queue(np.flatnonzero(active&~protected));full_pass=False
 while True:
  if not heap:
   if full_pass:break
   full_pass=True;queue(np.flatnonzero(active&~protected));continue
  _,k,version=heapq.heappop(heap)
  if not active[k] or serial[k]!=version:continue
  mask=active.copy();mask[k]=False;trial=surface(mask);candidate=abs(trial(q)-z);attempted+=1
  if candidate.max()<=error:
   ids=np.flatnonzero(active);index=int(np.flatnonzero(ids==k)[0]);near={index}
   # 削除に伴うDelaunay近傍の変化で、削除優先度を再計算する。
   for _ in range(2):near.update(int(v) for j in list(near) for v in model.links[model.off[j]:model.off[j+1]])
   active=mask;model=trial;residual=candidate;removed+=1;full_pass=False;queue(ids[list(near)])
   progress(stage='prune',points=int(active.sum())-4,removed=removed,attempted=attempted,max=float(residual.max()),seconds=time.monotonic()-started,active=active)
 # 最後は局所近傍の仮定に頼らず、現状の全候補を検証して削除不能を確認。
 residual=abs(model(q)-z)
 return active,model,dict(max=float(residual.max()),rmse=float(np.sqrt(np.mean(residual**2))),removed=removed,attempted=attempted,noFeasibleSingleRemoval=True)

def build(region,x,y,error=20,fine_error=10):
 started=time.monotonic();dst=OUT/region/f'{error:g}'/f'{x}-{y}';dst.mkdir(parents=True,exist_ok=True)
 if (dst/'selection.json').exists():return json.loads((dst/'selection.json').read_text())
 fit.OUT=OUT/'fine';fit.build(region,fine_error,x,y,source_error=12.5)
 src=fit.OUT/region/f'{fine_error:g}'/f'{x}-{y}';info=json.loads((src/'packet.json').read_text());info['error']=error
 points=np.load(src/'control-gradient.npz')['points'];side=info['side'];world=np.asarray(json.loads((fit.BASE/region/'source/region.json').read_text())['origin'])+np.asarray(info['origin']);half=side/2+250;lo=np.ceil((world-half)/STEP-.5).astype(int);hi=np.floor((world+half)/STEP-.5).astype(int);queries=[];targets=[]
 for ty in range(int(lo[1]//256),int(hi[1]//256)+1):
  for tx in range(int(lo[0]//256),int(hi[0]//256)+1):
   x0=max(int(lo[0]),tx*256);x1=min(int(hi[0])+1,(tx+1)*256);y0=max(int(lo[1]),ty*256);y1=min(int(hi[1])+1,(ty+1)*256);a=tile(tx,ty)[y0-ty*256:y1-ty*256,x0-tx*256:x1-tx*256];yy,xx=np.nonzero(np.isfinite(a));queries.append(np.column_stack(((xx+x0+.5)*STEP-world[0],(yy+y0+.5)*STEP-world[1])));targets.append(a[yy,xx])
 q=np.vstack(queries);z=np.concatenate(targets)
 if not len(z):raise ValueError('有効DEM画素がありません')
 # 余白と境界1画素の支持点は固定し、既存接合条件を変えない。
 protected=np.any(abs(points[:,:2])>=side/2-STEP,axis=1);protected[:4]=True;active=None
 if (dst/'prune-state.npz').exists():active=np.load(dst/'prune-state.npz')['active']
 def status(**values):
  mask=values.pop('active');temp=dst/'prune-state.tmp.npz';np.savez_compressed(temp,active=mask);temp.replace(dst/'prune-state.npz');write(dst/'progress.json',dict(region=region,x=x,y=y,error=error,initialPoints=len(points)-4,**values))
 write(dst/'progress.json',dict(stage='rank-candidates',initialPoints=len(points)-4))
 mask,model,audit=reduce_points(points,q,z,error,protected,status,active);p=points[mask]
 bounds=np.asarray(info['bounds']);n=max(16384,2**int(np.ceil(np.log2(len(p)*4))));samples=qmc.Sobol(2,scramble=True,seed=9124).random_base2(int(np.log2(n)))*(bounds[2:]-bounds[:2])+bounds[:2];c1=GlobalC1(p,strength=.01,queries=samples,surface=model);close_cells(c1.fast_model)
 meta={k:info[k] for k in ['region','error','x','y','side','origin','coreBounds','worldFieldBounds','halo']};meta.update(curve='Sibson-error-tested-point-removal',lpf=False,sourceGeometryBytes=len(p)*12,allDemPixelsAudited=True,auditedErrorTarget=error,auditedMaxError=audit['max'],maxErrorGuaranteed=False,auditScope='all valid original DEM pixel centers in tile plus 250m band; usual Sibson; C1/render/blending excluded')
 packet,out=pack(p,c1.gradient,bounds,meta,model=c1.fast_model,max_candidates=48,max_depth=11)
 if out['unsafeLeaves']:raise ValueError('unsafe GPU Voronoi leaves')
 audit.update(region=region,x=x,y=y,errorTarget=error,fineError=fine_error,initialPoints=len(points)-4,finalPoints=len(p)-4,fixedPoints=int(protected.sum())-4,validPixels=len(z),seconds=time.monotonic()-started,priority='leave-one-out control residual estimate; every acceptance audits all DEM pixels',globalMinimumNotGuaranteed=True)
 (dst/'packet.bin').write_bytes(packet);np.savez_compressed(dst/'control-gradient.npz',points=p,gradient=c1.gradient);out['pruningAudit']=audit;write(dst/'packet.json',out);write(dst/'selection.json',audit);write(dst/'progress.json',dict(stage='complete',**audit));return audit

def job(row):
 try:return build(*row)
 except Exception:
  region,x,y=row;write(OUT/region/'20'/f'{x}-{y}'/'failure.json',dict(message=traceback.format_exc()));raise

def run():
 rows=[(r,x,y) for r in ['fuji','yari','tsukuba'] for x,y in [(31,31),(32,31),(31,32),(32,32)]];OUT.mkdir(parents=True,exist_ok=True)
 lock=sqlite3.connect(OUT/'run-lock.sqlite3',timeout=0);lock.execute('BEGIN EXCLUSIVE')
 with ProcessPoolExecutor(max_workers=max(1,(os.cpu_count() or 2)//2)) as pool:
  result=[]
  for a in pool.map(job,rows):result.append(a);write(OUT/'report.json',dict(complete=len(result)==len(rows),results=result,total=len(rows)));print(json.dumps(a),flush=True)
if __name__=='__main__':run()
