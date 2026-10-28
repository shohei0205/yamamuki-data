"""細レベルの量子化XYZを固定して削減し、独立アンカー間を差分保存する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import json,sqlite3,zlib
from pathlib import Path
import numpy as np
from scipy.stats import qmc
from scipy.spatial import cKDTree
from free_mls import LocalSupport,operator,reference_values,SPAN
from mls_error_quality import peak_samples,requirements
from mls_downgrade import removal_order,choose_removals
from mls_quant_codec import encode_quant,decode_quant

def failed_locations(p,q,t,e):
 s=LocalSupport(p[:,:2],pre_smooth=False,floor_factor=.5)
 err=abs(operator(p[:,:2],q,s,curvature=1e-5)@p[:,2]-t);limit=np.where(np.linalg.norm(q,axis=1)<=2000,e*.5,e)
 return q[err>limit]

def measure(p,q,t,e):
 s=LocalSupport(p[:,:2],pre_smooth=False,floor_factor=.5)
 return requirements(operator(p[:,:2],q,s,curvature=1e-5)@p[:,2]-t,q,e)
def main():
 out=Path('build/mls-fixedxyz-prune-v4');out.mkdir(exist_ok=True)
 axis=np.linspace(-16000,16000,257);x,y=np.meshgrid(axis,axis)
 q=np.vstack((np.column_stack((x.ravel(),y.ravel())),(qmc.Sobol(2,scramble=True,seed=904).random_base2(14)-.5)*SPAN,peak_samples(12,935,1000)))
 h=np.vstack(((qmc.Sobol(2,scramble=True,seed=1904).random_base2(14)-.5)*SPAN,peak_samples(12,1935,1000)))
 with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:b=db.execute("SELECT coefficients FROM surfaces WHERE region='fuji' AND n=256").fetchone()[0]
 original=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261);truth=reference_values(original,q);ht=reference_values(original,h);report=[]
 with sqlite3.connect('build/mls-error-quality-v7/quality.sqlite3') as db,sqlite3.connect(out/'quality.sqlite3') as dest:
  dest.execute('CREATE TABLE IF NOT EXISTS levels(error REAL PRIMARY KEY,parent_error REAL,info TEXT,points BLOB)')
  for coarse,fine in ((200,100),(50,25)):
   b=db.execute("SELECT points FROM levels WHERE region='fuji' AND target_error=?",(fine,)).fetchone()[0];p=np.frombuffer(zlib.decompress(b),'<f4')[1:].reshape(-1,3).copy();p=decode_quant(encode_quant(p));keep=np.arange(len(p));proposal=int(len(p)*.25);history=[]
   for batch in range(12):
    current=p[keep];order,radii,protected=removal_order(original,current[:,:2],coarse);success=False
    for attempt in range(8):
     remove=choose_removals(current[:,:2],order,radii,protected,max(16,proposal));mask=np.ones(len(current),bool);mask[remove]=False;c=current[mask]
     audit=measure(c,q,truth,coarse*.8);held=measure(c,h,ht,coarse*.8) if audit['converged'] else None
     accepted=audit['converged'] and held['converged'];entry=dict(coarse=coarse,batch=batch,points=len(c),accepted=accepted,audit=audit,heldout=held);history.append(entry);print(json.dumps(entry),flush=True)
     if accepted:keep=keep[mask];success=True;proposal=max(16,int(len(keep)*.25));break
     # 誤差が超過した近傍を次の削減から保護し、他の領域の削減を試す。
     bad=failed_locations(c,q,truth,coarse*.8) if not audit['converged'] else failed_locations(c,h,ht,coarse*.8)
     if len(bad):
      near=cKDTree(current[:,:2]).query(bad,k=min(24,len(current)))[1];protected[np.asarray(near).ravel()]=True
     if attempt%2:proposal//=2
     if proposal<16:break
    (out/f'history-{coarse}.json').write_text(json.dumps(history,indent=2),encoding='utf-8')
    if not success:break
   added=np.ones(len(p),bool);added[keep]=False
   for e,data,parent in ((coarse,p[keep],None),(fine,p[added],coarse)):
    blob=encode_quant(data);restored=decode_quant(blob)
    if e==coarse:base=restored;allp=base
    else:allp=np.vstack((base,restored))
    audit=measure(allp,q,truth,e);held=measure(allp,h,ht,e)
    if not audit['converged'] or not held['converged']:raise ValueError('保存後に誤差条件を満たしません')
    info=dict(error=e,parent=parent,controls=len(allp),storedPoints=len(data),bytes=len(blob),audit=audit,heldout=held)
    dest.execute('INSERT INTO levels VALUES(?,?,?,?)',(e,parent,json.dumps(info),blob));dest.commit();report.append(info);print('LEVEL',json.dumps(info),flush=True)
 (out/'report.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
if __name__=='__main__':main()
