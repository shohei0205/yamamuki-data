"""生成した全制御点でも誤差評価し、高密度域の未拘束を減らす。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import json,sqlite3,zlib
from pathlib import Path
import numpy as np
from scipy.sparse.linalg import lsmr
from scipy.stats import qmc
from mls_generated_controls import generate,encode,decode
from mls_quant_codec import decode_quant
from free_mls import operator,LocalSupport,reference_values,SPAN
from mls_error_quality import peak_samples,requirements
out=Path('build/mls-generated-controls-v4');out.mkdir(exist_ok=True)
with sqlite3.connect('build/mls-generated-controls-v3/quality.sqlite3') as db:
 parent=decode_quant(db.execute('SELECT blob FROM levels WHERE error=50').fetchone()[0])
xy=generate(parent,2);points=np.vstack((parent[:,:2],xy));axis=np.linspace(-16000,16000,257);x,y=np.meshgrid(axis,axis)
q=np.vstack((points,np.column_stack((x.ravel(),y.ravel())),(qmc.Sobol(2,scramble=True,seed=904).random_base2(14)-.5)*SPAN,peak_samples(12,935,1000),(qmc.Sobol(2,scramble=True,seed=12904).random_base2(15)-.5)*SPAN,peak_samples(13,12935,1000)))
with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:b=db.execute("SELECT coefficients FROM surfaces WHERE region='fuji' AND n=256").fetchone()[0]
original=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261);truth=reference_values(original,q);support=LocalSupport(points,pre_smooth=False,floor_factor=2.);a=operator(points,q,support,curvature=1e-5);n=len(parent);design=a[:,n:];fixed=a[:,:n]@parent[:,2];z=np.load('build/mls-generated-controls-v3/fit-E25-r2.npz')['z'];limit=np.where(np.linalg.norm(q,axis=1)<=2000,12.5,25);weights=1/limit;best=None
for it in range(8):
 dz=lsmr(design.multiply(weights[:,None]),(truth-fixed-design@z)*weights,damp=.001,maxiter=600,atol=1e-7,btol=1e-7)[0];z+=dz;err=fixed+design@z-truth;score=float(np.max(abs(err)/limit))
 if best is None or score<best[0]:best=(score,z.copy());np.save(out/'z.npy',z)
 print(json.dumps(dict(iteration=it,normalizedMaxError=score)),flush=True)
 if score<.75:break
 weights=np.maximum(weights,np.clip(abs(err)/limit,1,32)**2/limit)
z=best[1];h=np.vstack(((qmc.Sobol(2,scramble=True,seed=13904).random_base2(15)-.5)*SPAN,peak_samples(13,13935,1000)));ht=reference_values(original,h);ah=operator(points,h,support,curvature=1e-5);trials=[];choices=[]
for step in (.125,1,2,4):
 blob=encode(parent,z,2,step);restored=np.vstack((parent,decode(parent,blob)));audit=requirements(a@restored[:,2]-truth,q,25);held=requirements(ah@restored[:,2]-ht,h,25);info=dict(error=25,parent=50,step=step,bytes=len(blob),controls=len(points),added=len(xy),audit=audit,heldout=held);trials.append(info)
 if audit['converged'] and held['converged']:choices.append((len(blob),info,blob))
print(json.dumps(trials),flush=True);(out/'trials.json').write_text(json.dumps(trials,indent=2),encoding='utf-8')
if choices:
 _,info,blob=min(choices,key=lambda v:v[0])
 with sqlite3.connect('build/mls-generated-controls-v3/quality.sqlite3') as src,sqlite3.connect(out/'quality.sqlite3') as dest:
  for name in ('metadata','levels'):
   dest.execute(src.execute('SELECT sql FROM sqlite_master WHERE name=?',(name,)).fetchone()[0])
   rows=src.execute('SELECT * FROM '+name).fetchall()
   for row in rows:
    if name=='levels' and row[0]==25:continue
    dest.execute('INSERT INTO '+name+' VALUES('+','.join('?' for _ in row)+')',row)
  dest.execute('INSERT INTO levels VALUES(?,?,?,?,?)',(25,50,'MLG1',json.dumps(info),blob))
 (out/'report.json').write_text(json.dumps(info,indent=2),encoding='utf-8')
