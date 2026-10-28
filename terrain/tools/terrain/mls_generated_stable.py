"""生成点を全採用したまま、MLSの支持範囲と復号予測を改善する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import json,sqlite3,zlib,lzma,hashlib,time
from pathlib import Path
import numpy as np
from scipy.spatial import Delaunay,cKDTree
from scipy.sparse.linalg import lsmr
from scipy.stats import qmc
from mls_generated_controls import HEADER
from mls_quant_codec import decode_quant
from free_mls import operator,reference_values,SPAN
from mls_error_quality import peak_samples,requirements
class DirectSupport:
 def __init__(self,xy):self.tree=cKDTree(xy);self.floor=2*SPAN/np.sqrt(len(xy))
 def __call__(self,q):return np.maximum(self.floor,self.tree.query(q,k=24)[0][:,-1]*1.5)
def generate_linear(parent,rounds):
 if rounds not in (1,2):raise ValueError('段数が不正')
 p=parent.astype('<f4').copy();n=len(p)
 for _ in range(rounds):
  t=Delaunay(p[:,:2]).simplices;e=np.unique(np.sort(np.vstack((t[:,[0,1]],t[:,[1,2]],t[:,[2,0]])),axis=1),axis=0);p=np.vstack((p,((p[e[:,0]].astype(float)+p[e[:,1]])*.5).astype('<f4')))
 return p[n:]
def encode(parent,z,rounds,step):
 if not np.isfinite(step) or step<=0:raise ValueError('残差刻みが不正')
 p=generate_linear(parent,rounds);v=np.rint((z-p[:,2])/step)
 if len(v)!=len(p) or not np.isfinite(v).all() or np.any(abs(v)>np.iinfo(np.int32).max):raise ValueError('残差が範囲外')
 v=v.astype('<i4');raw=v.view('u1').reshape(-1,4).T.copy().tobytes();return HEADER.pack(b'MLG2',rounds,len(p),step,hashlib.sha256(p[:,:2].copy().tobytes()).digest())+lzma.compress(raw,preset=6)
def decode(parent,b):
 if len(b)<HEADER.size:raise ValueError('ヘッダー不足')
 magic,r,n,step,digest=HEADER.unpack_from(b)
 if magic!=b'MLG2' or step<=0 or not np.isfinite(step):raise ValueError('形式不正')
 p=generate_linear(parent,r)
 if n!=len(p) or digest!=hashlib.sha256(p[:,:2].copy().tobytes()).digest():raise ValueError('生成位置が不一致')
 raw=lzma.decompress(b[HEADER.size:])
 if len(raw)!=4*n:raise ValueError('係数数が不一致')
 p[:,2]=(p[:,2].astype(float)+np.frombuffer(raw,'u1').reshape(4,n).T.copy().view('<i4').reshape(n)*step).astype('<f4');return p

def main():
 out=Path('build/mls-generated-stable-v1');out.mkdir(exist_ok=True);path=out/'quality.sqlite3'
 if path.exists():raise ValueError('既存出力を保持する')
 axis=np.linspace(-16000,16000,257);x,y=np.meshgrid(axis,axis);baseq=np.vstack((np.column_stack((x.ravel(),y.ravel())),(qmc.Sobol(2,scramble=True,seed=904).random_base2(14)-.5)*SPAN,peak_samples(12,935,1000),(qmc.Sobol(2,scramble=True,seed=12904).random_base2(15)-.5)*SPAN,peak_samples(13,12935,1000)))
 h=np.vstack(((qmc.Sobol(2,scramble=True,seed=14904).random_base2(15)-.5)*SPAN,peak_samples(13,14935,1000)))
 with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:b=db.execute("SELECT coefficients FROM surfaces WHERE region='fuji' AND n=256").fetchone()[0]
 original=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261);ht=reference_values(original,h);report=[]
 with sqlite3.connect('build/mls-hybrid-selected-v2/quality.sqlite3') as src,sqlite3.connect(path) as dest:
  dest.execute('CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT) WITHOUT ROWID');dest.execute('CREATE TABLE levels(error REAL PRIMARY KEY,parent_error REAL,codec TEXT,info TEXT,blob BLOB)')
  for k,v in src.execute('SELECT * FROM metadata'):dest.execute('INSERT INTO metadata VALUES(?,?)',(k,v))
  for k,v in dict(childSupport='direct-k24-scale1.5-floor2',generatedRule='edge-midpoints-all-linearXYZ',distributable=False).items():dest.execute('INSERT OR REPLACE INTO metadata VALUES(?,?)',(k,json.dumps(v)))
  for coarse,fine in ((200,100),(50,25)):
   raw,b=src.execute('SELECT info,points FROM levels WHERE error=?',(coarse,)).fetchone();parent=decode_quant(b);dest.execute('INSERT INTO levels VALUES(?,?,?,?,?)',(coarse,None,'MLQ1',raw,b));dest.commit();added=generate_linear(parent,1);points=np.vstack((parent[:,:2],added[:,:2]));q=np.vstack((baseq,points));truth=reference_values(original,q);support=DirectSupport(points);a=operator(points,q,support,curvature=1e-5);ah=operator(points,h,support,curvature=1e-5);n=len(parent);design=a[:,n:];fixed=a[:,:n]@parent[:,2]
   old=Path(f'build/mls-generated-controls-v2/fit-E{fine}-r1.npz');z=np.load(old)['z'].copy();limit=np.where(np.linalg.norm(q,axis=1)<=2000,fine*.5,fine);weights=1/limit;best=None
   for it in range(8):
    delta=lsmr(design.multiply(weights[:,None]),(truth-fixed-design@z)*weights,damp=.001,maxiter=600,atol=1e-7,btol=1e-7)[0];z+=delta;err=fixed+design@z-truth;score=float(np.max(abs(err)/limit))
    if best is None or score<best[0]:best=(score,z.copy())
    print(json.dumps(dict(error=fine,iteration=it,normalizedError=score)),flush=True)
    if score<.8:break
    weights=np.maximum(weights,np.clip(abs(err)/limit,1,32)**2/limit)
   z=best[1];np.save(out/f'z-E{fine}.npy',z);trials=[];choices=[]
   for step in (.125,1,2,4,8,16):
    b=encode(parent,z,1,step);t=time.perf_counter();r=decode(parent,b);decode_ms=(time.perf_counter()-t)*1000;p=np.vstack((parent,r));audit=requirements(a@p[:,2]-truth,q,fine);held=requirements(ah@p[:,2]-ht,h,fine);info=dict(error=fine,parent=coarse,added=len(r),controls=len(p),step=step,bytes=len(b),decodeMs=decode_ms,audit=audit,validation=held);trials.append(info)
    if audit['converged'] and held['converged']:choices.append((len(b),info,b))
   (out/f'trials-E{fine}.json').write_text(json.dumps(trials,indent=2),encoding='utf-8')
   if choices:
    _,info,b=min(choices,key=lambda v:v[0]);dest.execute('INSERT INTO levels VALUES(?,?,?,?,?)',(fine,coarse,'MLG2',json.dumps(info),b));dest.commit();report.append(info);print('SELECTED',json.dumps(info),flush=True)
   else:print('UNMET',fine,flush=True)
  result=dict(levels=report,passed=len(report)==2,blobBytes=dest.execute('SELECT sum(length(blob)) FROM levels').fetchone()[0]);(out/'report.json').write_text(json.dumps(result,indent=2),encoding='utf-8');print(json.dumps(result),flush=True)
if __name__=='__main__':main()
