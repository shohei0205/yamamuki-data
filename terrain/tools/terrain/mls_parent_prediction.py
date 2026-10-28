"""自由配置MLSの追加制御点を親曲面予測と残差係数で圧縮する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import json,sqlite3,zlib,itertools
from pathlib import Path
import numpy as np
from scipy.stats import qmc
from scipy.spatial import cKDTree
from free_mls import LocalSupport,operator,reference_values,SPAN
from mls_error_quality import peak_samples,requirements
from mls_quant_codec import encode_quant,decode_quant

def predict(parent,xy):
 if not len(xy):return np.empty(0)
 support=LocalSupport(parent[:,:2],pre_smooth=False,floor_factor=.5)
 return operator(parent[:,:2],xy,support,curvature=1e-5)@parent[:,2]
def predict_plane(parent,xy):
 if not len(xy):return np.empty(0)
 distance,ids=cKDTree(parent[:,:2]).query(xy,k=min(12,len(parent)))
 near=parent[ids];delta=near[:,:,:2]-xy[:,None,:];scale=np.maximum(distance[:,-1],1.)
 basis=np.concatenate((np.ones((*distance.shape,1)),delta/scale[:,None,None]),axis=2)
 weights=1/(.01+(distance/scale[:,None])**2);matrix=np.einsum('nki,nk,nkj->nij',basis,weights,basis)
 matrix+=np.eye(3)[None,:,:]*1e-9;rhs=np.einsum('nki,nk,nk->ni',basis,weights,near[:,:,2])
 return np.linalg.solve(matrix,rhs[:,:,None])[:,0,0]
def encode_prediction(parent,added,step):
 p=np.asarray(added).copy();p[:,2]=p[:,2]-predict(parent,p[:,:2])
 return b'MLP1'+encode_quant(p,zstep=step)
def decode_prediction(parent,blob):
 if blob[:4] not in (b'MLP1',b'MLP2'):raise ValueError('予測形式が不正')
 p=decode_quant(blob[4:]);prediction=predict_plane(parent,p[:,:2]) if blob[:4]==b'MLP2' else predict(parent,p[:,:2])
 p[:,2]=(p[:,2].astype(float)+prediction).astype('<f4');return p

def main():
 out=Path('build/mls-parent-prediction-v3');out.mkdir(exist_ok=True);path=out/'quality.sqlite3'
 if path.exists():raise ValueError('既存出力は保持する')
 axis=np.linspace(-16000,16000,257);x,y=np.meshgrid(axis,axis)
 q=np.vstack((np.column_stack((x.ravel(),y.ravel())),(qmc.Sobol(2,scramble=True,seed=904).random_base2(14)-.5)*SPAN,peak_samples(12,935,1000)))
 h=np.vstack(((qmc.Sobol(2,scramble=True,seed=7904).random_base2(15)-.5)*SPAN,peak_samples(13,7935,1000)))
 with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:b=db.execute("SELECT coefficients FROM surfaces WHERE region='fuji' AND n=256").fetchone()[0]
 truth=reference_values(np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261),q);ht=reference_values(np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261),h);models={};trials=[];selected=[]
 with sqlite3.connect('build/mls-hybrid-selected-v2/quality.sqlite3') as src,sqlite3.connect(path) as dest:
  dest.execute('CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT) WITHOUT ROWID');dest.execute('CREATE TABLE levels(error REAL PRIMARY KEY,parent_error REAL,codec TEXT,info TEXT,blob BLOB)')
  for k,v in src.execute('SELECT * FROM metadata'):dest.execute('INSERT INTO metadata VALUES(?,?)',(k,v))
  dest.execute('INSERT INTO metadata VALUES(?,?)',('childCodec',json.dumps('MLP1')))
  for e,parent,raw,b in src.execute('SELECT * FROM levels ORDER BY error DESC'):
   added=decode_quant(b)
   if parent is None:
    models[e]=added;dest.execute('INSERT INTO levels VALUES(?,?,?,?,?)',(e,None,'MLQ1',raw,b));selected.append(dict(error=e,bytes=len(b),codec='MLQ1'));continue
   base=models[parent];p=np.vstack((base,added));s=LocalSupport(p[:,:2],pre_smooth=False,floor_factor=.5);a=operator(p[:,:2],q,s,curvature=1e-5);ah=operator(p[:,:2],h,s,curvature=1e-5);candidates=[]
   prediction=predict(base,added[:,:2]);plane_prediction=predict_plane(base,added[:,:2]);baseline_z=p[:,2].copy()
   for mode,step in itertools.product(('parent-prediction','local-plane','absolute'),(.125,.5,1,2,4,8,16,32,64)):
    residual=added.copy()
    if mode=='parent-prediction':residual[:,2]-=prediction
    if mode=='local-plane':residual[:,2]-=plane_prediction
    blob=(b'MLP1' if mode=='parent-prediction' else b'MLP2' if mode=='local-plane' else b'')+encode_quant(residual,zstep=step)
    restored=decode_prediction(base,blob) if mode!='absolute' else decode_quant(blob)
    resolved=np.vstack((base,restored));audit=requirements(a@resolved[:,2]-truth,q,e)
    item=dict(error=e,parent=parent,mode=mode,step=step,bytes=len(blob),baselineBytes=len(b),audit=audit)
    trials.append(item)
    if audit['converged']:candidates.append((len(blob),item,blob,resolved))
   # 元のXYの順序を維持するため、encode_quantのMorton順が変わらないことも検証する。
   chosen=None;best_modes={}
   for size,item,blob,resolved in sorted(candidates,key=lambda v:v[0]):
    if item['mode'] in best_modes:continue
    if not np.array_equal(resolved[:,:2],p[:,:2]):raise ValueError('制御点XYの順序が変わりました')
    item['heldout']=requirements(ah@resolved[:,2]-ht,h,e);item['maxControlZChange']=float(abs(resolved[:,2]-baseline_z).max());item['surfaceRmseChange']=float(np.sqrt(np.mean((a@(resolved[:,2]-baseline_z))**2)))
    if item['heldout']['converged']:
     best_modes[item['mode']]=(item,blob,resolved)
     if chosen is None:chosen=(item,blob,resolved)
   if chosen is None:raise ValueError('品質条件を満たす候補がありません')
   item,blob,resolved=chosen;models[e]=resolved;selected.append(item);dest.execute('INSERT INTO levels VALUES(?,?,?,?,?)',(e,parent,'MLP1' if item['mode']=='parent-prediction' else 'MLP2' if item['mode']=='local-plane' else 'MLQ1',json.dumps(item),blob));dest.commit();print(json.dumps({mode:v[0] for mode,v in best_modes.items()}),flush=True)
 result=dict(levels=selected,blobBytes=sum(v['bytes'] for v in selected),baselineBytes=169560,sqliteBytes=path.stat().st_size,validationPoints=len(h))
 (out/'report.json').write_text(json.dumps(result,indent=2),encoding='utf-8');(out/'trials.json').write_text(json.dumps(trials,indent=2),encoding='utf-8');print(json.dumps(result),flush=True)
if __name__=='__main__':main()
