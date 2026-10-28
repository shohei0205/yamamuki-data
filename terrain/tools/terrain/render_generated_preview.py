"""改善MLSと参照曲面のスクリーンショット用データを作る。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import sys,sqlite3,zlib,json
from pathlib import Path
import numpy as np
from mls_generated_stable import decode,DirectSupport
from mls_quant_codec import decode_quant
from free_mls import operator,reference_values,SPAN
out=Path('build/mls-generated-preview');out.mkdir(exist_ok=True)
with sqlite3.connect('build/mls-generated-stable-v1/quality.sqlite3') as db:
 parent=decode_quant(db.execute('SELECT blob FROM levels WHERE error=50').fetchone()[0]);blob=db.execute('SELECT blob FROM levels WHERE error=25').fetchone()[0];p=np.vstack((parent,decode(parent,blob)))
n=512;axis=np.linspace(-16000,16000,n+1);x,y=np.meshgrid(axis,axis);q=np.column_stack((x.ravel(),y.ravel()))
with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:b=db.execute("SELECT coefficients FROM surfaces WHERE region='fuji' AND n=256").fetchone()[0]
original=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261)
for name,h in [('reference',reference_values(original,q)),('improved',operator(p[:,:2],q,DirectSupport(p[:,:2]),curvature=1e-5)@p[:,2])]:
 h=h.reshape(n+1,n+1);gy,gx=np.gradient(h,SPAN/n,edge_order=2);np.stack((h,gx,gy),axis=2).astype('<f4').tofile(out/f'{name}.bin')
(out/'info.json').write_text(json.dumps(dict(n=n,span=SPAN,controls=len(p),added=len(p)-len(parent),bytes=len(blob),quality=25)),encoding='utf-8')
print(str(out.resolve()))
