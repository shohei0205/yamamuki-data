"""槍ヶ岳の再配置25m版を親として12.5m目標を追加する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import json,sqlite3,zlib,time
from pathlib import Path
import numpy as np
from scipy.sparse.linalg import lsmr
from scipy.stats import qmc
from mls_generated_stable import generate_linear,DirectSupport,encode,decode
from free_mls import operator,reference_values,SPAN
from mls_error_quality import peak_samples,requirements

def main():
 out=Path('build/mls-parent-child-layout-E12p5-v1');out.mkdir(exist_ok=True)
 source=Path('build/mls-parent-child-layout-v1/iteration-2');root=np.fromfile(source/'parent.f32','<f4').reshape(-1,3);parent=np.vstack((root,decode(root,(source/'residual-step8.mlg2').read_bytes())));parent.tofile(out/'parent.f32')
 added=generate_linear(parent,1);xy=np.vstack((parent[:,:2],added[:,:2]));support=DirectSupport(xy);n=len(parent)
 with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:b=db.execute("SELECT coefficients FROM surfaces WHERE region='yari' AND n=256").fetchone()[0]
 original=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261);axis=np.linspace(-16000,16000,257);x,y=np.meshgrid(axis,axis);q=np.vstack((xy,np.column_stack((x.ravel(),y.ravel())),(qmc.Sobol(2,scramble=True,seed=19004).random_base2(14)-.5)*SPAN,peak_samples(13,19452,1500)));truth=reference_values(original,q);a=operator(xy,q,support,curvature=1e-5);design=a[:,n:];fixed=a[:,:n]@parent[:,2];z=reference_values(original,added[:,:2]);limit=np.where(np.linalg.norm(q,axis=1)<=2000,6.25,12.5);weights=1/limit;best=None;started=time.perf_counter()
 for it in range(6):
  delta=lsmr(design.multiply(weights[:,None]),(truth-fixed-design@z)*weights,damp=.003,maxiter=500,atol=1e-7,btol=1e-7)[0];z+=delta;err=fixed+design@z-truth;score=float(np.max(abs(err)/limit))
  if best is None or score<best[0]:best=(score,z.copy());np.save(out/'child-z.npy',z)
  print(json.dumps(dict(stage='fit',iteration=it,controls=len(xy),normalizedError=score,seconds=time.perf_counter()-started)),flush=True)
  if score<.8:break
  weights=np.maximum(weights,np.clip(abs(err)/limit,1,24)**2/limit)
 z=best[1];v=np.vstack(((qmc.Sobol(2,scramble=True,seed=19904).random_base2(15)-.5)*SPAN,peak_samples(13,19935,1000)));vt=reference_values(original,v);av=operator(xy,v,support,curvature=1e-5);trials=[];choice=None
 for step in (.5,1,2,4,8):
  blob=encode(parent,z,1,step);p=np.vstack((parent,decode(parent,blob)));audit=requirements(a@p[:,2]-truth,q,12.5);validation=requirements(av@p[:,2]-vt,v,12.5);score=max(audit['measuredMaxError']/12.5,audit['measuredPeakMaxError']/6.25,validation['measuredMaxError']/12.5,validation['measuredPeakMaxError']/6.25);info=dict(targetError=12.5,parentTargetError=25,parentControls=n,controls=len(p),added=len(added),step=step,bytes=len(blob),audit=audit,validation=validation,score=score);trials.append(info);key=(not(audit['converged'] and validation['converged']),len(blob) if audit['converged'] and validation['converged'] else score)
  if choice is None or key<choice[0]:choice=(key,info,blob,p)
 _,info,blob,p=choice;(out/'residual.mlg2').write_bytes(blob);(out/'trials.json').write_text(json.dumps(trials,indent=2),encoding='utf-8')
 h=np.vstack(((qmc.Sobol(2,scramble=True,seed=20904).random_base2(15)-.5)*SPAN,peak_samples(13,20935,1000)));measurement=requirements(operator(xy,h,support,curvature=1e-5)@p[:,2]-reference_values(original,h),h,12.5);info['finalValidation']=measurement;(out/'info.json').write_text(json.dumps(info,indent=2),encoding='utf-8');print('RESULT',json.dumps(info),flush=True)
 # 表示だけの格子へ評価する。保存する形状はMLG2の残差係数。
 view=Path('build/mls-generated-preview/yari-levels');view.mkdir(exist_ok=True);n=512;axis=np.linspace(-16000,16000,n+1);x,y=np.meshgrid(axis,axis);grid=np.column_stack((x.ravel(),y.ravel()));height=(operator(xy,grid,support,curvature=1e-5)@p[:,2]).reshape(n+1,n+1);gy,gx=np.gradient(height,SPAN/n,edge_order=2);np.stack((height,gx,gy),axis=2).astype('<f4').tofile(view/'level-12.5.bin');(view/'level-12.5.json').write_text(json.dumps(dict(n=n,span=SPAN,**info,**measurement),indent=2),encoding='utf-8');print('PREVIEW_READY',flush=True)
if __name__=='__main__':main()
