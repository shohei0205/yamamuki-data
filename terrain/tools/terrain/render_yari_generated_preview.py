"""槍ヶ岳でも全中間点生成MLSを生成・計測して比較表示する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import sqlite3,zlib,json
from pathlib import Path
import numpy as np
from scipy.stats import qmc
from scipy.sparse.linalg import lsmr
from mls_generated_stable import generate_linear,DirectSupport,encode,decode
from free_mls import operator,reference_values,SPAN
from mls_error_quality import peak_samples,requirements
out=Path('build/mls-generated-preview/yari');out.mkdir(exist_ok=True)
with sqlite3.connect('build/free-mls-v3/mls.sqlite3') as db:raw,b=db.execute("SELECT info,points FROM surfaces WHERE region='yari' AND budget=8192").fetchone()
parent=np.frombuffer(zlib.decompress(b),'<f4')[1:].reshape(-1,3).copy();added=generate_linear(parent,1);xy=np.vstack((parent[:,:2],added[:,:2]));support=DirectSupport(xy)
with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:b=db.execute("SELECT coefficients FROM surfaces WHERE region='yari' AND n=256").fetchone()[0]
original=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261);axis=np.linspace(-16000,16000,257);x,y=np.meshgrid(axis,axis);q=np.vstack((xy,np.column_stack((x.ravel(),y.ravel())),peak_samples(13,452,1500)))
truth=reference_values(original,q);a=operator(xy,q,support,curvature=1e-5);n=len(parent);design=a[:,n:];fixed=a[:,:n]@parent[:,2];z=reference_values(original,added[:,:2]);limit=np.where(np.linalg.norm(q,axis=1)<=2000,12.5,25);weights=1/limit;best=None
for it in range(6):
 delta=lsmr(design.multiply(weights[:,None]),(truth-fixed-design@z)*weights,damp=.001,maxiter=600,atol=1e-7,btol=1e-7)[0];z+=delta;err=fixed+design@z-truth;score=float(np.max(abs(err)/limit))
 if best is None or score<best[0]:best=(score,z.copy())
 print(json.dumps(dict(stage='fit',iteration=it,normalizedMaxError=score)),flush=True)
 if score<.8:break
 weights=np.maximum(weights,np.clip(abs(err)/limit,1,32)**2/limit)
z=best[1];h=np.vstack(((qmc.Sobol(2,scramble=True,seed=16904).random_base2(15)-.5)*SPAN,peak_samples(13,16935,1000)));ht=reference_values(original,h);ah=operator(xy,h,support,curvature=1e-5);choices=[];trials=[]
for step in (1,4,8,16):
 blob=encode(parent,z,1,step);p=np.vstack((parent,decode(parent,blob)));m=requirements(ah@p[:,2]-ht,h,25);trials.append(dict(step=step,bytes=len(blob),**m));choices.append((not m['converged'],len(blob) if m['converged'] else m['measuredMaxError'],blob,p,m))
_,_,blob,p,measurement=min(choices,key=lambda v:v[:2]);(out/'residual.mlg2').write_bytes(blob);parent.tofile(out/'parent.f32');(out/'trials.json').write_text(json.dumps(trials,indent=2),encoding='utf-8')
n=512;axis=np.linspace(-16000,16000,n+1);x,y=np.meshgrid(axis,axis);g=np.column_stack((x.ravel(),y.ravel()))
for name,height in [('reference',reference_values(original,g)),('improved',operator(p[:,:2],g,support,curvature=1e-5)@p[:,2])]:
 height=height.reshape(n+1,n+1);gy,gx=np.gradient(height,SPAN/n,edge_order=2);np.stack((height,gx,gy),axis=2).astype('<f4').tofile(out/f'{name}.bin')
info=dict(n=n,span=SPAN,controls=len(p),added=len(added),bytes=len(blob),targetError=25,**measurement);(out/'info.json').write_text(json.dumps(info,indent=2),encoding='utf-8');print(json.dumps(info),flush=True)
