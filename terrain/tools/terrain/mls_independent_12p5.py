"""槍ヶ岳12.5m目標を同じ全点配置で独立適合・独立保存する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import json,sqlite3,zlib,time
from pathlib import Path
import numpy as np
from scipy.sparse.linalg import lsmr
from scipy.stats import qmc
from mls_generated_stable import generate_linear,DirectSupport
from mls_quant_codec import encode_quant,decode_quant
from free_mls import operator,reference_values,SPAN
from mls_error_quality import peak_samples,requirements

def main():
 out=Path('build/mls-independent-yari-E12p5-v1');out.mkdir(exist_ok=True)
 if (out/'info.json').exists():raise ValueError('既存出力を保持する')
 parent=np.fromfile('build/mls-residual-curve-yari-v2/parent.f32','<f4').reshape(-1,3);added=generate_linear(parent,1);xy=np.vstack((parent[:,:2],added[:,:2]));canonical=decode_quant(encode_quant(np.column_stack((xy,np.zeros(len(xy)))),xy=.0625));xy=canonical[:,:2].copy()
 if len(np.unique(xy,axis=0))!=len(xy):raise ValueError('位置量子化で点が重複')
 support=DirectSupport(xy)
 with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:b=db.execute("SELECT coefficients FROM surfaces WHERE region='yari' AND n=256").fetchone()[0]
 original=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261);axis=np.linspace(-16000,16000,257);x,y=np.meshgrid(axis,axis);q=np.vstack((xy,np.column_stack((x.ravel(),y.ravel())),(qmc.Sobol(2,scramble=True,seed=25004).random_base2(15)-.5)*SPAN,peak_samples(13,25452,1500)));truth=reference_values(original,q);print(json.dumps(dict(stage='operator',controls=len(xy),samples=len(q))),flush=True)
 a=operator(xy,q,support,curvature=1e-5);z=reference_values(original,xy);limits=np.where(np.linalg.norm(q,axis=1)<=2000,6.25,12.5);weights=1/limits;initial_error=a@z-truth;best=(float(np.max(abs(initial_error)/limits)),z.copy());started=time.perf_counter()
 for it in range(8):
  delta=lsmr(a.multiply(weights[:,None]),(truth-a@z)*weights,damp=.003,maxiter=600,atol=1e-7,btol=1e-7)[0];z+=delta;err=a@z-truth;score=float(np.max(abs(err)/limits))
  if score<best[0]:best=(score,z.copy());np.save(out/'z.npy',z)
  print(json.dumps(dict(stage='fit',iteration=it,normalizedError=score,seconds=time.perf_counter()-started)),flush=True)
  if score<.8:break
  weights=np.maximum(weights,np.clip(abs(err)/limits,1,24)**2/limits)
 z=best[1];np.save(out/'z.npy',z)
 v=np.vstack(((qmc.Sobol(2,scramble=True,seed=25904).random_base2(15)-.5)*SPAN,peak_samples(13,25935,1000)));vt=reference_values(original,v);av=operator(xy,v,support,curvature=1e-5);trials=[];chosen=None
 for step in (.125,.5,1,2,4,8):
  p=np.column_stack((xy,z)).astype('<f4');blob=encode_quant(p,xy=.0625,zstep=step);restored=decode_quant(blob)
  if not np.array_equal(restored[:,:2],xy):raise ValueError('独立復号のXYが不一致')
  audit=requirements(a@restored[:,2]-truth,q,12.5);validation=requirements(av@restored[:,2]-vt,v,12.5);score=max(audit['measuredMaxError']/12.5,audit['measuredPeakMaxError']/6.25,validation['measuredMaxError']/12.5,validation['measuredPeakMaxError']/6.25);info=dict(targetError=12.5,parentTargetError=None,parentControls=0,controls=len(xy),added=0,step=step,bytes=len(blob),xyStep=.0625,reconstruction='independent-MLS',audit=audit,validation=validation,score=score);trials.append(info);passed=audit['converged'] and validation['converged'];key=(not passed,len(blob) if passed else score)
  if chosen is None or key<chosen[0]:chosen=(key,info,blob,restored)
 _,info,blob,p=chosen;(out/'model.mlq1').write_bytes(blob);(out/'trials.json').write_text(json.dumps(trials,indent=2),encoding='utf-8');h=np.vstack(((qmc.Sobol(2,scramble=True,seed=26904).random_base2(15)-.5)*SPAN,peak_samples(13,26935,1000)));measurement=requirements(operator(xy,h,support,curvature=1e-5)@p[:,2]-reference_values(original,h),h,12.5);commonq=np.vstack(((qmc.Sobol(2,scramble=True,seed=24904).random_base2(15)-.5)*SPAN,peak_samples(13,24935,1000)));common=requirements(operator(xy,commonq,support,curvature=1e-5)@p[:,2]-reference_values(original,commonq),commonq,12.5);info.update(finalValidation=measurement,commonValidation=common);(out/'info.json').write_text(json.dumps(info,indent=2),encoding='utf-8');print('RESULT',json.dumps(info),flush=True)
 metadata=dict(region='yari',span=SPAN,format='MLQ1',parent=None,support='direct-k24-scale1.5-floor2',curvaturePenalty=1e-5,distributable=False);(out/'metadata.json').write_text(json.dumps(metadata,indent=2),encoding='utf-8')
 view=Path('build/mls-generated-preview/yari-levels');n=512;axis=np.linspace(-16000,16000,n+1);x,y=np.meshgrid(axis,axis);g=np.column_stack((x.ravel(),y.ravel()));height=(operator(xy,g,support,curvature=1e-5)@p[:,2]).reshape(n+1,n+1);gy,gx=np.gradient(height,SPAN/n,edge_order=2);np.stack((height,gx,gy),axis=2).astype('<f4').tofile(view/'level-12.5-independent.bin');(view/'level-12.5-independent.json').write_text(json.dumps(dict(n=n,span=SPAN,**info,**common,validationLabel='共通検証'),indent=2),encoding='utf-8');print('PREVIEW_READY',flush=True)
if __name__=='__main__':main()
