"""親曲面を保持し、全生成中間点のMLS残差曲面を加算する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import json,sqlite3,zlib,lzma,hashlib,time
from pathlib import Path
import numpy as np
from scipy.sparse.linalg import lsmr
from scipy.stats import qmc
from mls_generated_stable import generate_linear,DirectSupport,decode as decode_parent
from mls_generated_controls import HEADER
from free_mls import operator,reference_values,SPAN
from mls_error_quality import peak_samples,requirements

def encode(parent,coefficients,step):
 if step<=0 or not np.isfinite(step):raise ValueError('残差刻みが不正')
 xy=generate_linear(parent,1)[:,:2].copy();v=np.rint(np.asarray(coefficients)/step)
 if len(v)!=len(xy) or not np.isfinite(v).all() or np.any(abs(v)>np.iinfo(np.int32).max):raise ValueError('残差係数が不正')
 raw=v.astype('<i4').view('u1').reshape(-1,4).T.copy().tobytes()
 return HEADER.pack(b'MLR1',1,len(xy),step,hashlib.sha256(xy.tobytes()).digest())+lzma.compress(raw,preset=6)
def decode(parent,blob):
 if len(blob)<HEADER.size:raise ValueError('ヘッダー不足')
 magic,rounds,n,step,digest=HEADER.unpack_from(blob)
 if magic!=b'MLR1' or rounds!=1 or step<=0 or not np.isfinite(step):raise ValueError('形式不正')
 xy=generate_linear(parent,1)[:,:2].copy()
 if len(xy)!=n or hashlib.sha256(xy.tobytes()).digest()!=digest:raise ValueError('位置の再生成が不一致')
 raw=lzma.decompress(blob[HEADER.size:])
 if len(raw)!=4*n:raise ValueError('係数数が不一致')
 coefficients=np.frombuffer(raw,'u1').reshape(4,n).T.copy().view('<i4').reshape(n).astype(float)*step
 return xy,coefficients

def main():
 out=Path('build/mls-residual-curve-yari-v1');out.mkdir(exist_ok=True)
 if (out/'info.json').exists():raise ValueError('既存の実験結果を保持する')
 old=Path('build/mls-parent-child-layout-v1/iteration-2');root=np.fromfile(old/'parent.f32','<f4').reshape(-1,3);parent=np.vstack((root,decode_parent(root,(old/'residual-step8.mlg2').read_bytes())));parent.tofile(out/'parent.f32');xy=generate_linear(parent,1)[:,:2].copy();ps=DirectSupport(parent[:,:2]);rs=DirectSupport(xy)
 with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:b=db.execute("SELECT coefficients FROM surfaces WHERE region='yari' AND n=256").fetchone()[0]
 original=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261);axis=np.linspace(-16000,16000,257);x,y=np.meshgrid(axis,axis);q=np.vstack((parent[:,:2],xy,np.column_stack((x.ravel(),y.ravel())),(qmc.Sobol(2,scramble=True,seed=21004).random_base2(14)-.5)*SPAN,peak_samples(13,21452,1500)));truth=reference_values(original,q)
 print(json.dumps(dict(stage='parent-evaluation',parent=len(parent),residualControls=len(xy),samples=len(q))),flush=True)
 parent_values=operator(parent[:,:2],q,ps,curvature=1e-5)@parent[:,2];target=truth-parent_values;design=operator(xy,q,rs,curvature=1e-5);coeff=np.zeros(len(xy));limit=np.where(np.linalg.norm(q,axis=1)<=2000,6.25,12.5);weights=1/limit;best=(float(np.max(abs(target)/limit)),coeff.copy());started=time.perf_counter()
 for it in range(8):
  delta=lsmr(design.multiply(weights[:,None]),(target-design@coeff)*weights,damp=.003,maxiter=600,atol=1e-7,btol=1e-7)[0];coeff+=delta;err=parent_values+design@coeff-truth;score=float(np.max(abs(err)/limit))
  if score<best[0]:best=(score,coeff.copy());np.save(out/'coefficients.npy',coeff)
  print(json.dumps(dict(stage='fit',iteration=it,normalizedError=score,seconds=time.perf_counter()-started)),flush=True)
  if score<.8:break
  weights=np.maximum(weights,np.clip(abs(err)/limit,1,24)**2/limit)
 coeff=best[1];np.save(out/'coefficients.npy',coeff)
 v=np.vstack(((qmc.Sobol(2,scramble=True,seed=21904).random_base2(15)-.5)*SPAN,peak_samples(13,21935,1000)));vt=reference_values(original,v);pv=operator(parent[:,:2],v,ps,curvature=1e-5)@parent[:,2];av=operator(xy,v,rs,curvature=1e-5);parent_measure=requirements(pv-vt,v,12.5);trials=[];choice=None
 for step in (.125,.5,1,2,4,8):
  blob=encode(parent,coeff,step);_,restored=decode(parent,blob);audit=requirements(parent_values+design@restored-truth,q,12.5);validation=requirements(pv+av@restored-vt,v,12.5);score=max(audit['measuredMaxError']/12.5,audit['measuredPeakMaxError']/6.25,validation['measuredMaxError']/12.5,validation['measuredPeakMaxError']/6.25);item=dict(targetError=12.5,parentTargetError=25,parentControls=len(parent),controls=len(parent)+len(xy),added=len(xy),residualControls=len(xy),step=step,bytes=len(blob),reconstruction='parent-plus-residual-MLS',audit=audit,validation=validation,parentValidation=parent_measure,score=score);trials.append(item);key=(not(audit['converged'] and validation['converged']),len(blob) if audit['converged'] and validation['converged'] else score)
  if choice is None or key<choice[0]:choice=(key,item,blob,restored)
 _,info,blob,coeff=choice;(out/'residual.mlr1').write_bytes(blob);(out/'trials.json').write_text(json.dumps(trials,indent=2),encoding='utf-8')
 h=np.vstack(((qmc.Sobol(2,scramble=True,seed=22904).random_base2(15)-.5)*SPAN,peak_samples(13,22935,1000)));ht=reference_values(original,h);ph=operator(parent[:,:2],h,ps,curvature=1e-5)@parent[:,2];child=ph+operator(xy,h,rs,curvature=1e-5)@coeff;measurement=requirements(child-ht,h,12.5);info.update(finalValidation=measurement,parentFinalValidation=requirements(ph-ht,h,25));(out/'info.json').write_text(json.dumps(info,indent=2),encoding='utf-8');print('RESULT',json.dumps(info),flush=True)
 # 描画用の派生データ。形状保存に固定格子を使わない。
 view=Path('build/mls-generated-preview/yari-levels');n=512;axis=np.linspace(-16000,16000,n+1);x,y=np.meshgrid(axis,axis);g=np.column_stack((x.ravel(),y.ravel()));height=(operator(parent[:,:2],g,ps,curvature=1e-5)@parent[:,2]+operator(xy,g,rs,curvature=1e-5)@coeff).reshape(n+1,n+1);gy,gx=np.gradient(height,SPAN/n,edge_order=2);np.stack((height,gx,gy),axis=2).astype('<f4').tofile(view/'level-12.5-residual.bin');(view/'level-12.5-residual.json').write_text(json.dumps(dict(n=n,span=SPAN,**info,**measurement),indent=2),encoding='utf-8');print('PREVIEW_READY',flush=True)
if __name__=='__main__':main()
