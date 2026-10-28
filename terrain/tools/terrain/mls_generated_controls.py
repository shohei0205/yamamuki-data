"""親Delaunay辺の中間点を全採用し、Z残差だけを保存する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import argparse,json,sqlite3,zlib,lzma,struct,hashlib,time
from pathlib import Path
import numpy as np
from scipy.spatial import Delaunay
from scipy.sparse.linalg import lsmr
from scipy.stats import qmc
from free_mls import LocalSupport,operator,reference_values,SPAN
from mls_error_quality import peak_samples,requirements
from mls_quant_codec import decode_quant
from mls_parent_prediction import predict
HEADER=struct.Struct('<4sBIf32s')
def generate(parent,rounds):
 if rounds not in (1,2,3):raise ValueError('生成段数が範囲外です')
 points=parent[:,:2].astype('<f4').copy();n=len(points)
 for _ in range(rounds):
  tri=Delaunay(points).simplices;edges=np.sort(np.vstack((tri[:,[0,1]],tri[:,[1,2]],tri[:,[2,0]])),axis=1);edges=np.unique(edges,axis=0)
  mid=((points[edges[:,0]].astype(float)+points[edges[:,1]])*.5).astype('<f4')
  points=np.vstack((points,mid))
 if len(np.unique(points,axis=0))!=len(points):raise ValueError('生成位置が重複しました')
 return points[n:]
def encode(parent,z,rounds,step):
 if not np.isfinite(step) or step<=0:raise ValueError('残差刻みが不正')
 xy=generate(parent,rounds)
 if len(z)!=len(xy):raise ValueError('生成点数が一致しません')
 residual=np.rint((z-predict(parent,xy))/step)
 if not np.isfinite(residual).all() or np.max(abs(residual))>np.iinfo(np.int32).max:raise ValueError('残差が範囲外')
 v=residual.astype('<i4');raw=v.view('u1').reshape(-1,4).T.copy().tobytes()
 return HEADER.pack(b'MLG1',rounds,len(v),step,hashlib.sha256(xy.tobytes()).digest())+lzma.compress(raw,preset=6)
def decode(parent,blob):
 if len(blob)<HEADER.size:raise ValueError('ヘッダー不足')
 magic,rounds,n,step,digest=HEADER.unpack_from(blob)
 if magic!=b'MLG1' or not 1<=rounds<=3 or not np.isfinite(step) or step<=0:raise ValueError('不正な形式')
 xy=generate(parent,rounds)
 if len(xy)!=n or hashlib.sha256(xy.tobytes()).digest()!=digest:raise ValueError('親からの位置再生成が一致しません')
 raw=lzma.decompress(blob[HEADER.size:])
 if len(raw)!=4*n:raise ValueError('残差数が一致しません')
 z=np.frombuffer(raw,'u1').reshape(4,n).T.copy().view('<i4').reshape(n).astype(float)*step+predict(parent,xy)
 return np.column_stack((xy,z)).astype('<f4')
def main():
 parser=argparse.ArgumentParser();parser.add_argument('--fine-rounds',type=int,default=1,choices=[1,2]);parser.add_argument('--output',type=Path,default=Path('build/mls-generated-controls-v2'));args=parser.parse_args()
 out=args.output;out.mkdir(exist_ok=True);path=out/'quality.sqlite3'
 if path.exists():raise ValueError('既存出力を保持します')
 axis=np.linspace(-16000,16000,257);x,y=np.meshgrid(axis,axis);q=np.vstack((np.column_stack((x.ravel(),y.ravel())),(qmc.Sobol(2,scramble=True,seed=904).random_base2(14)-.5)*SPAN,peak_samples(12,935,1000)))
 h=np.vstack(((qmc.Sobol(2,scramble=True,seed=10904).random_base2(15)-.5)*SPAN,peak_samples(13,10935,1000)))
 with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:b=db.execute("SELECT coefficients FROM surfaces WHERE region='fuji' AND n=256").fetchone()[0]
 original=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261);q=np.vstack((q,h));truth=reference_values(original,q);ht=reference_values(original,h);results=[]
 with sqlite3.connect('build/mls-hybrid-selected-v2/quality.sqlite3') as src,sqlite3.connect(path) as dest:
  dest.execute('CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT) WITHOUT ROWID');dest.execute('CREATE TABLE levels(error REAL PRIMARY KEY,parent_error REAL,codec TEXT,info TEXT,blob BLOB)')
  for k,v in src.execute('SELECT * FROM metadata'):dest.execute('INSERT INTO metadata VALUES(?,?)',(k,v))
  dest.execute("UPDATE metadata SET value='2.0' WHERE key='supportFloorFactor'")
  dest.execute('INSERT INTO metadata VALUES(?,?)',('parentSupportFloorFactor','0.5'))
  dest.execute('INSERT INTO metadata VALUES(?,?)',('generatedRule',json.dumps('sorted-Delaunay-edge-midpoint-all-v1')))
  dest.execute('INSERT INTO metadata VALUES(?,?)',('distributable','false'))
  for coarse,fine in ((200,100),(50,25)):
   raw,b=src.execute('SELECT info,points FROM levels WHERE error=?',(coarse,)).fetchone();parent=decode_quant(b);dest.execute('INSERT INTO levels VALUES(?,?,?,?,?)',(coarse,None,'MLQ1',raw,b));dest.commit();candidates=[]
   for rounds in ((args.fine_rounds,) if fine==25 else (1,2)):
    # 2段生成も全点を使う。中間段は保存レベルではなく位置生成の手順。
    started=time.monotonic();xy=generate(parent,rounds);p=np.vstack((parent[:,:2],xy));support=LocalSupport(p,pre_smooth=False,floor_factor=2.);a=operator(p,q,support,curvature=1e-5);ah=operator(p,h,support,curvature=1e-5);n=len(parent);design=a[:,n:];fixed=a[:,:n]@parent[:,2];pred=predict(parent,xy);z=reference_values(original,xy);limit=np.where(np.linalg.norm(q,axis=1)<=2000,fine*.5,fine);weights=1/limit;best=None
    for it in range(20):
     dz=lsmr(design.multiply(weights[:,None]),(truth-fixed-design@z)*weights,damp=.001,maxiter=800,atol=1e-7,btol=1e-7)[0];z+=dz;err=fixed+design@z-truth;score=float(np.max(abs(err)/limit))
     if best is None or score<best[0]:best=(score,z.copy())
     print(json.dumps(dict(stage='fit-iteration',error=fine,rounds=rounds,iteration=it,normalizedMaxError=score)),flush=True)
     if score<.8:break
     weights=np.maximum(weights,np.clip(abs(err)/limit,1,32)**2/limit)
    z=best[1];np.savez(out/f'fit-E{fine}-r{rounds}.npz',z=z)
    fitted=requirements(fixed+design@z-truth,q,fine);entry=dict(error=fine,parent=coarse,rounds=rounds,parentControls=n,added=len(xy),controls=len(p),fit=fitted,seconds=time.monotonic()-started);results.append(entry);print('FIT',json.dumps(entry),flush=True)
    for step in (.125,1,4,8,16,32):
     blob=encode(parent,z,rounds,step);restored=decode(parent,blob);resolved=np.vstack((parent,restored));audit=requirements(a@resolved[:,2]-truth,q,fine);held=requirements(ah@resolved[:,2]-ht,h,fine)
     item=dict(**entry,step=step,bytes=len(blob),audit=audit,heldout=held)
     if audit['converged'] and held['converged']:candidates.append((len(blob),item,blob))
    if candidates:break
   if candidates:
    _,info,blob=min(candidates,key=lambda v:v[0]);dest.execute('INSERT INTO levels VALUES(?,?,?,?,?)',(fine,coarse,'MLG1',json.dumps(info),blob));dest.commit();print('SELECTED',json.dumps(info),flush=True)
   else:print('UNMET',fine,flush=True)
   (out/'report.json').write_text(json.dumps(results,indent=2),encoding='utf-8')
if __name__=='__main__':main()
