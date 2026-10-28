"""自由位置の点を高密度に保存し、参照曲面の再現精度を優先する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import json,sqlite3,zlib,time,gc
from pathlib import Path
import numpy as np
from scipy.stats import qmc
from free_mls import operator,reference_values,SPAN
from mls_generated_stable import DirectSupport
from mls_quant_codec import encode_quant,decode_quant
from mls_error_quality import peak_samples,requirements

def evaluate(xy,z,query,support,chunk=8192):
 values=np.empty(len(query))
 for i in range(0,len(query),chunk):values[i:i+chunk]=operator(xy,query[i:i+chunk],support,curvature=1e-5,tree=support.tree)@z
 return values

def reference(original,xy):
 values=np.empty(len(xy))
 for i in range(0,len(xy),16384):values[i:i+16384]=reference_values(original,xy[i:i+16384])
 return values

def main():
 out=Path('build/mls-high-precision-yari-v2');out.mkdir(exist_ok=True)
 if (out/'info.json').exists():raise ValueError('既存結果を保持する')
 with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:b=db.execute("SELECT coefficients FROM surfaces WHERE region='yari' AND n=256").fetchone()[0]
 original=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261);axis=np.linspace(-16000,16000,513);x,y=np.meshgrid(axis,axis);audit=np.vstack((np.column_stack((x.ravel(),y.ravel())),(qmc.Sobol(2,scramble=True,seed=27904).random_base2(16)-.5)*SPAN,peak_samples(14,27935,1000)));truth=reference(original,audit);reports=[];best=None
 for requested in (8388608,):
  started=time.perf_counter();folder=out/f'n-{requested}';folder.mkdir(exist_ok=True);rng=np.random.default_rng(27004);xy=np.rint(rng.uniform(-16000,16000,(requested,2))*32)/32;xy[:4]=[[-16000,-16000],[-16000,16000],[16000,-16000],[16000,16000]];xy=np.unique(xy.astype('<f4'),axis=0);print(json.dumps(dict(stage='sample',requested=requested,controls=len(xy))),flush=True)
  z=reference(original,xy).astype('<f4');p=np.column_stack((xy,z)).astype('<f4');del xy,z;gc.collect();print(json.dumps(dict(stage='encode',controls=len(p),seconds=time.perf_counter()-started)),flush=True)
  blob=encode_quant(p,xy=.03125,zstep=.0078125);(folder/'model.mlq1').write_bytes(blob);p=decode_quant(blob);support=DirectSupport(p[:,:2]);actual=evaluate(p[:,:2],p[:,2],audit,support);measurement=requirements(actual-truth,audit,1.);info=dict(targetError=1.,qualityLabel='高精度・独立（サイズ制限なし）',requestedControls=requested,controls=len(p),added=0,bytes=len(blob),xyStep=.03125,zStep=.0078125,reconstruction='independent-MLS',generation='dense-irregular-reference-samples',audit=measurement,auditCount=len(audit),seconds=time.perf_counter()-started);reports.append(info);(folder/'info.json').write_text(json.dumps(info,indent=2),encoding='utf-8');print(json.dumps(info),flush=True)
  score=max(measurement['measuredMaxError'],measurement['measuredPeakMaxError']*2)
  if best is None or score<best[0]:best=(score,folder,info)
  del p,support,actual,blob;gc.collect()
 _,folder,info=best;blob=(folder/'model.mlq1').read_bytes();p=decode_quant(blob);support=DirectSupport(p[:,:2]);finalq=np.vstack(((qmc.Sobol(2,scramble=True,seed=28904).random_base2(17)-.5)*SPAN,peak_samples(15,28935,1000)));measurement=requirements(evaluate(p[:,:2],p[:,2],finalq,support)-reference(original,finalq),finalq,1.);na=info['auditCount'];nv=len(finalq);audit_measure=info['audit'];summary=dict(measuredMaxError=max(audit_measure['measuredMaxError'],measurement['measuredMaxError']),measuredPeakMaxError=max(audit_measure['measuredPeakMaxError'],measurement['measuredPeakMaxError']),measuredRmse=float(np.sqrt((na*audit_measure['measuredRmse']**2+nv*measurement['measuredRmse']**2)/(na+nv))),converged=audit_measure['converged'] and measurement['converged']);info=dict(info,selectedFolder=folder.name,finalValidation=measurement,validationSummary=summary,evaluationCount=na+nv,stages=reports);(out/'info.json').write_text(json.dumps(info,indent=2),encoding='utf-8');metadata=dict(region='yari',span=SPAN,format='MLQ1',parent=None,support='direct-k24-scale1.5-floor2',curvaturePenalty=1e-5,reference='saved-B-spline-n256',distributable=False);(out/'metadata.json').write_text(json.dumps(metadata,indent=2),encoding='utf-8');print('RESULT',json.dumps(info),flush=True)
 # 比較画面向けの派生データを作る。保存形状は自由配置点。
 view=Path('build/mls-generated-preview/yari-levels');n=512;g=audit[:(n+1)**2];height=evaluate(p[:,:2],p[:,2],g,support).reshape(n+1,n+1);gy,gx=np.gradient(height,SPAN/n,edge_order=2);np.stack((height,gx,gy),axis=2).astype('<f4').tofile(view/'level-high-precision.bin');(view/'level-high-precision.json').write_text(json.dumps(dict(n=n,span=SPAN,**info,**summary,validationLabel='評価＋独立検証'),indent=2),encoding='utf-8');print('PREVIEW_READY',flush=True)
if __name__=='__main__':main()
