"""自由配置制御点のZを制限付きで適合し、自然近傍の曲率集中を抑える比較。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json,time
import numpy as np
from scipy.stats import qmc
from scipy.sparse import vstack,eye,diags
from scipy.optimize import lsq_linear
from weighted_sibson import WeightedSibson
from natural_c1_128 import ROOT,RUN as BASE,GlobalC1,close_cells,pack,write
from audit_reduced_c1_128 import dem
from continuous_dem import STEP
OUT=ROOT/'build/natural-raw-c1-128-regularized-v3'
def build(region,x,y):
 src=BASE/region/'12.5'/f'{x}-{y}';dst=OUT/region/'12.5'/f'{x}-{y}';dst.mkdir(parents=True,exist_ok=True)
 if (dst/'selection.json').exists():return
 info=json.loads((src/'packet.json').read_text());data=np.load(src/'control-gradient.npz');p=data['points'].copy();old=p.copy();basis=WeightedSibson(p);bounds=np.asarray(info['bounds']);side=info['side'];halo=info['halo'];real=np.all(np.abs(p[:,:2])<=side/2+halo+1,axis=1);free=real.copy()
 # 10m以上の局所山頂・谷底を固定して、ピークを丸める解を抑える。
 protected=0
 for i in np.flatnonzero(real):
  ids=basis.model.links[basis.model.off[i]:basis.model.off[i+1]];dz=p[ids,2]-p[i,2]
  if len(dz) and (np.all(dz<=0) or np.all(dz>=0)) and abs(dz).max()>10:free[i]=False;protected+=1
 q=qmc.Sobol(2,scramble=True,seed=7331).random_base2(12)*(bounds[2:]-bounds[:2]-40)+bounds[:2]+20;d=12.;a=basis.basis(q);lap=(basis.basis(q+[d,0])+basis.basis(q-[d,0])+basis.basis(q+[0,d])+basis.basis(q-[0,d])-a*4).tocsr();norm=np.sqrt(np.asarray(lap.power(2).sum(axis=1)).ravel());lap=diags(1/np.maximum(norm,.05))@lap
 ids=np.flatnonzero(free);scale=.5;system=vstack((eye(len(ids)),lap[:,ids]*scale),format='csr');right=np.r_[np.zeros(len(ids)),-scale*(lap@p[:,2])];fit=lsq_linear(system,right,bounds=(-6.25,6.25),tol=1e-5,lsmr_tol=1e-5,max_iter=80);assert np.isfinite(fit.x).all();p[ids,2]=np.round((p[ids,2]+fit.x)/.125)*.125
 # 元DEMとの標高誤差を悪化させない範囲へ変更量を戻す。
 world=np.asarray(json.loads((BASE/region/'source/region.json').read_text())['origin'])+np.asarray(info['origin']);auditq=qmc.Sobol(2,scramble=True,seed=65428).random_base2(14)*side-side/2;worldq=(np.floor((auditq+world)/STEP)+.5)*STEP;q2=worldq-world;ref=dem(worldq);v=np.isfinite(ref);before=basis(q2);limit=max(12.5,float(abs(before[v]-ref[v]).max()));delta=p[:,2]-old[:,2];accepted=0.
 # 最大誤差を決める少数地点のためにタイル全体の調整をゼロへ戻さず、
 # その地点に影響する制御点だけを誤差の許容帯へ射影する。
 audit_basis=basis.basis(q2);matrix=audit_basis[:,ids].tocsr();change=delta[ids].copy();valid_ids=np.flatnonzero(v)
 for iteration in range(80):
  predicted=before+matrix@change;residual=predicted-ref;bad=valid_ids[abs(residual[valid_ids])>limit-.126]
  if not len(bad):break
  adjusted=False
  for row in bad:
   start,end=matrix.indptr[row:row+2];cols=matrix.indices[start:end];weights=matrix.data[start:end];norm=float(weights@weights)
   if norm<1e-14:continue
   value=float(before[row]+weights@change[cols]-ref[row]);excess=value-np.clip(value,-limit+.126,limit-.126)
   if abs(excess)>1e-8:change[cols]-=weights*(excess/norm);change=np.clip(change,-6.25,6.25);adjusted=True
  if not adjusted:break
 delta[ids]=change
 for fraction in [1.,.5,.25,.125,0.]:
  p[:,2]=old[:,2]+np.round(delta*fraction/.125)*.125;candidate=WeightedSibson(p)(q2)
  if float(abs(candidate[v]-ref[v]).max())<=limit+1e-7:accepted=fraction;break
 queries=qmc.Sobol(2,scramble=True,seed=9124).random_base2(14)*(bounds[2:]-bounds[:2])+bounds[:2];model=GlobalC1(p,strength=.01,queries=queries);close_cells(model.fast_model)
 meta={k:info[k] for k in ['region','error','x','y','side','origin','coreBounds','worldFieldBounds','halo']};meta.update(curve='Sibson-curvature-constrained-controls',lpf=False,maxErrorGuaranteed=False,sourceGeometryBytes=len(p)*12)
 packet,out=pack(p,model.gradient,bounds,meta,model=model.fast_model,max_candidates=32,max_depth=8)
 if out['unsafeLeaves']:raise ValueError('unsafe cells')
 after=model.model(q2);audit=dict(sampledOnly=True,samples=int(v.sum()),beforeMax=float(abs(before[v]-ref[v]).max()),afterMax=float(abs(after[v]-ref[v]).max()),beforeRmse=float(np.sqrt(np.mean((before[v]-ref[v])**2))),afterRmse=float(np.sqrt(np.mean((after[v]-ref[v])**2))),zChangeMax=float(abs(p[:,2]-old[:,2]).max()),protectedExtrema=protected,xyUnchanged=True,acceptedFraction=accepted,errorLimit=limit,solverSuccess=bool(fit.success),beforeCurvature=float(np.linalg.norm(lap@old[:,2])),afterCurvature=float(np.linalg.norm(lap@p[:,2])))
 (dst/'packet.bin').write_bytes(packet);np.savez_compressed(dst/'control-gradient.npz',points=p,gradient=model.gradient);out['regularizedAudit']=audit;write(dst/'packet.json',out);write(dst/'selection.json',audit);print(region,x,y,audit,flush=True)
def run():
 OUT.mkdir(exist_ok=True);done=0
 for region in ['fuji','yari','tsukuba']:
  for y in range(30,34):
   for x in range(30,34):
    build(region,x,y);done+=1;write(OUT/'progress.json',dict(done=done,total=48,complete=done==48))
if __name__=='__main__':run()
