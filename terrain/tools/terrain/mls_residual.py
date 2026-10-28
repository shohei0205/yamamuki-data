"""親曲面を維持し、追加MLSで残差だけを表現する比較。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import json,sqlite3,zlib,time
from pathlib import Path
import numpy as np
from scipy.sparse.linalg import lsmr
from scipy.stats import qmc
from free_mls import LocalSupport,operator,reference_values,SPAN
from mls_error_quality import peak_samples,requirements
from mls_xyz_delta import encode,decode

def main():
 out=Path('build/mls-residual-v1');out.mkdir(exist_ok=True)
 axis=np.linspace(-16000,16000,257);x,y=np.meshgrid(axis,axis)
 q=np.vstack((np.column_stack((x.ravel(),y.ravel())),(qmc.Sobol(2,scramble=True,seed=904).random_base2(14)-.5)*SPAN,peak_samples(12,935,1000),(qmc.Sobol(2,scramble=True,seed=1904).random_base2(14)-.5)*SPAN,peak_samples(12,1935,1000)))
 hq=np.vstack(((qmc.Sobol(2,scramble=True,seed=2904).random_base2(14)-.5)*SPAN,peak_samples(12,2935,1000)))
 with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:b=db.execute("SELECT coefficients FROM surfaces WHERE region='fuji' AND n=256").fetchone()[0]
 original=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261);truth=reference_values(original,q);ht=reference_values(original,hq)
 with sqlite3.connect('build/mls-downgrade-v2/quality.sqlite3') as db:rows=db.execute("SELECT target_error,points FROM levels WHERE region='fuji' ORDER BY target_error DESC").fetchall()
 results=[]
 # 200/100 は独立、50 を独立アンカーとして25の残差を比較する。
 for start in (0,2):
  base=np.zeros(len(q));hb=np.zeros(len(hq));prior=set();total=0;layers=[]
  for j,(e,b) in enumerate(rows[start:]):
   if j>2:break
   data=np.frombuffer(zlib.decompress(b),'<f4')[1:].reshape(-1,3).copy();ids=np.load(f'build/mls-downgrade-v2/lineage-fuji-E{e:g}.npy');mask=np.array([int(v) not in prior for v in ids]);p=data[mask].copy()
   if j:
    corners=data[np.all(abs(data[:,:2])==16000,axis=1)];p=np.vstack((p,corners))
   support=LocalSupport(p[:,:2],pre_smooth=False,floor_factor=.5);a=operator(p[:,:2],q,support,curvature=1e-5);h=operator(p[:,:2],hq,support,curvature=1e-5)
   target=truth-base;z=np.zeros(len(p)) if j else p[:,2].astype(float);limit=np.where(np.linalg.norm(q,axis=1)<=2000,e*.5,e);weights=1/limit;best=None
   for it in range(10):
    dz=lsmr(a.multiply(weights[:,None]),(target-a@z)*weights,damp=.001,maxiter=800,atol=1e-7,btol=1e-7)[0];z+=dz;err=a@z-target;score=float(np.max(abs(err)/limit))
    if best is None or score<best[0]:best=(score,z.copy())
    if score<.85:break
    weights=np.maximum(weights,np.clip(abs(err)/limit,1,32)**2/limit)
   p[:,2]=best[1].astype('<f4');blob=encode(p);np.save(out/f'layer-{start}-{j}.npy',decode(blob),allow_pickle=False);base+=a@p[:,2];hb+=h@p[:,2];total+=len(blob);prior=set(map(int,ids))
   result=dict(start=start,level=e,layer=j,points=len(p),bytes=len(blob),cumulativeBytes=total,audit=requirements(base-truth,q,e),heldout=requirements(hb-ht,hq,e));results.append(result)
   print(json.dumps(result),flush=True);(out/'report.json').write_text(json.dumps(results,indent=2),encoding='utf-8')
if __name__=='__main__':main()
