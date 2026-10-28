"""生成点の配置と復号処理の費用を切り分ける。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import json,sqlite3,zlib,time,lzma
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree
from scipy.stats import qmc
from mls_generated_controls import generate,decode,HEADER
from mls_quant_codec import decode_quant
from mls_parent_prediction import predict
from free_mls import LocalSupport,operator,reference_values,SPAN
from mls_error_quality import peak_samples
out=Path('build/mls-generated-diagnosis-v1');out.mkdir(exist_ok=True)
with sqlite3.connect('build/mls-generated-controls-v3/quality.sqlite3') as db:
 parent=decode_quant(db.execute('SELECT blob FROM levels WHERE error=50').fetchone()[0]);blob=db.execute('SELECT blob FROM levels WHERE error=25').fetchone()[0]
profile={};t=time.perf_counter();xy=generate(parent,2);profile['generateXYMs']=(time.perf_counter()-t)*1000
t=time.perf_counter();prediction=predict(parent,xy);profile['parentMLSPredictionMs']=(time.perf_counter()-t)*1000
t=time.perf_counter();raw=lzma.decompress(blob[HEADER.size:]);z=np.frombuffer(raw,'u1').reshape(4,len(xy)).T.copy().view('<i4').reshape(-1)*4+prediction;profile['decompressAndAddMs']=(time.perf_counter()-t)*1000
points=np.vstack((parent,np.column_stack((xy,z))));q=np.vstack(((qmc.Sobol(2,scramble=True,seed=12904).random_base2(15)-.5)*SPAN,peak_samples(13,12935,1000)))
with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:b=db.execute("SELECT coefficients FROM surfaces WHERE region='fuji' AND n=256").fetchone()[0]
truth=reference_values(np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261),q);support=LocalSupport(points[:,:2],pre_smooth=False,floor_factor=2.);a=operator(points[:,:2],q,support,curvature=1e-5);error=a@points[:,2]-truth;dist,ids=cKDTree(points[:,:2]).query(q,k=12);amplification=np.asarray(abs(a).sum(axis=1)).ravel();counts=np.diff(a.indptr);bad=np.argsort(abs(error))[-20:][::-1];stats=[]
for i in bad:
 local=points[ids[i],:2]-q[i];cov=local.T@local;ev=np.linalg.eigvalsh(cov)
 stats.append(dict(xy=q[i].tolist(),error=float(error[i]),nearestMeters=float(dist[i,0]),twelfthMeters=float(dist[i,-1]),supportMeters=float(support(q[i:i+1])[0]),neighbors=int(counts[i]),weightL1=float(amplification[i]),covarianceRatio=float(ev[0]/max(ev[1],1e-20))))
result=dict(profile=profile,controls=len(points),added=len(xy),medianNearestMeters=float(np.median(dist[:,0])),medianTwelfthMeters=float(np.median(dist[:,-1])),medianWeightL1=float(np.median(amplification)),bad=stats)
(out/'report.json').write_text(json.dumps(result,indent=2),encoding='utf-8');print(json.dumps(result))
