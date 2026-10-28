"""独立アンカーと共通XYZ最適化を比較する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import json,sqlite3,zlib,itertools,lzma,time
from pathlib import Path
import numpy as np
from scipy import sparse
from scipy.sparse.linalg import lsmr
from scipy.stats import qmc
from free_mls import LocalSupport,operator,reference_values,SPAN
from mls_error_quality import peak_samples,requirements
from mls_xyz_delta import encode,decode

def main():
 out=Path('build/mls-hybrid-v1');out.mkdir(exist_ok=True)
 axis=np.linspace(-16000,16000,257);x,y=np.meshgrid(axis,axis)
 q=np.vstack((np.column_stack((x.ravel(),y.ravel())),(qmc.Sobol(2,scramble=True,seed=904).random_base2(14)-.5)*SPAN,peak_samples(12,935,1000)))
 hold=np.vstack(((qmc.Sobol(2,scramble=True,seed=1904).random_base2(14)-.5)*SPAN,peak_samples(12,1935,1000)))
 with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:b=db.execute("SELECT coefficients FROM surfaces WHERE region='fuji' AND n=256").fetchone()[0]
 original=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261);truth=reference_values(original,q);holdtruth=reference_values(original,hold)
 models=[]
 with sqlite3.connect('build/mls-downgrade-v2/quality.sqlite3') as db:
  for e,b in db.execute("SELECT target_error,points FROM levels WHERE region='fuji' ORDER BY target_error DESC"):
   p=np.frombuffer(zlib.decompress(b),'<f4')[1:].reshape(-1,3).copy();ids=np.load(f'build/mls-downgrade-v2/lineage-fuji-E{e:g}.npy');support=LocalSupport(p[:,:2],pre_smooth=False,floor_factor=.5)
   models.append(dict(e=e,p=p,ids=ids,a=operator(p[:,:2],q,support,curvature=1e-5),h=operator(p[:,:2],hold,support,curvature=1e-5)))
 report=[]
 # 全ての連続グループを検証し、その後に独立アンカーの組合せを選ぶ。
 for start in range(4):
  for stop in range(start+1,5):
   group=models[start:stop];fine=group[-1];ids=fine['ids'];mapping={int(v):i for i,v in enumerate(ids)};n=len(ids)
   mats=[];hmats=[];targets=[];limits=[]
   for m in group:
    ix=np.array([mapping[int(v)] for v in m['ids']]);a=m['a'].tocoo();h=m['h'].tocoo()
    mats.append(sparse.csr_matrix((a.data,(a.row,ix[a.col])),shape=(len(q),n)));hmats.append(sparse.csr_matrix((h.data,(h.row,ix[h.col])),shape=(len(hold),n)))
    targets.append(truth);limits.append(np.where(np.linalg.norm(q,axis=1)<=2000,m['e']*.5,m['e']))
   design=sparse.vstack(mats,format='csr');target=np.concatenate(targets);limit=np.concatenate(limits);z=fine['p'][:,2].astype(float);best=None;weights=1/limit;beg=time.monotonic()
   for iteration in range(8 if len(group)>1 else 0):
    delta=lsmr(design.multiply(weights[:,None]),(target-design@z)*weights,damp=.001,maxiter=600,atol=1e-6,btol=1e-6)[0];candidate=z+delta;err=design@candidate-target;score=float(np.max(abs(err)/limit))
    if best is None or score<best[0]:best=(score,candidate.copy())
    z=candidate;weights=np.maximum(weights,np.clip(abs(err)/limit,1,16)**2/limit)
    if score<.97:break
   if best is not None:z=best[1]
   z=z.astype('<f4');measure=[];chunks=[];previous=set()
   for m,a,h in zip(group,mats,hmats):
    ix=np.array([mapping[int(v)] for v in m['ids']]);p=m['p'].copy();p[:,2]=z[ix];new=np.array([int(v) not in previous for v in m['ids']]);previous=set(map(int,m['ids']));blob=encode(p[new]);chunks.append(blob)
    measure.append(dict(error=m['e'],audit=requirements(a@z-truth,q,m['e']),heldout=requirements(h@z-holdtruth,hold,m['e']),added=int(new.sum()),bytes=len(blob)))
   name=f'{start}-{stop}';np.savez(out/f'group-{name}.npz',ids=ids,z=z)
   item=dict(start=start,stop=stop,seconds=time.monotonic()-beg,bytes=sum(map(len,chunks)),levels=measure,passed=all(v['audit']['converged'] and v['heldout']['converged'] for v in measure));report.append(item)
   (out/'groups.json').write_text(json.dumps(report,indent=2),encoding='utf-8');print(json.dumps(item),flush=True)
 choices=[]
 for mask in range(8):
  cuts=[0]+[i+1 for i in range(3) if mask&(1<<i)]+[4];groups=[next(v for v in report if v['start']==a and v['stop']==b) for a,b in zip(cuts,cuts[1:])]
  choices.append(dict(cuts=cuts,bytes=sum(v['bytes'] for v in groups),passed=all(v['passed'] for v in groups),maxDependencies=max(b-a-1 for a,b in zip(cuts,cuts[1:]))))
 (out/'strategies.json').write_text(json.dumps(sorted(choices,key=lambda v:v['bytes']),indent=2),encoding='utf-8')
if __name__=='__main__':main()
