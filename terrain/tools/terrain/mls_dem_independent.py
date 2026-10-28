"""元PNG DEM基準で槍ヶ岳の独立MLSを生成し、全元画素で検証する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import argparse,json,time,gc,sqlite3,zlib,hashlib
from pathlib import Path
import numpy as np
from scipy.sparse.linalg import lsmr
from scipy.stats import qmc
from raw_dem_reference import RawDem,center_for,STEP,SPAN
from mls_high_precision import evaluate
from mls_generated_stable import DirectSupport
from mls_quant_codec import encode_quant,decode_quant
from mls_error_quality import peak_samples,requirements

def native_sample(source,stride):
 lo=np.ceil((source.center-SPAN/2)/STEP-.5).astype(int);hi=np.floor((source.center+SPAN/2)/STEP-.5).astype(int);ix=np.arange(lo[0],hi[0]+1,stride);iy=np.arange(lo[1],hi[1]+1,stride);x,y=np.meshgrid((ix+.5)*STEP-source.center[0],(iy+.5)*STEP-source.center[1]);z=source.grid[np.ix_(iy-source.top,ix-source.left)].ravel();valid=np.isfinite(z);return np.column_stack((x.ravel(),y.ravel()))[valid],z[valid]
def main():
 parser=argparse.ArgumentParser();parser.add_argument('--resume-fit',action='store_true');args=parser.parse_args()
 out=Path('build/mls-dem-independent-yari-v1');out.mkdir(exist_ok=True)
 if (out/'info.json').exists():raise ValueError('既存結果を保持する')
 source=RawDem('build/terrain-cache',center_for('yari'))
 if (out/'source-tiles.json').exists() and json.loads((out/'source-tiles.json').read_text())!=source.manifest:raise ValueError('元DEMが前回と一致しません')
 (out/'source-tiles.json').write_text(json.dumps(source.manifest,indent=2),encoding='utf-8')
 # 同じ131,033点のXYで、基準だけを元DEMへ変更する。
 old=json.loads(Path('build/mls-independent-yari-E12p5-v1/info.json').read_text());xy=decode_quant(Path('build/mls-independent-yari-E12p5-v1/model.mlq1').read_bytes())[:,:2].copy();support=DirectSupport(xy);native,znative=native_sample(source,8);extra=np.vstack((xy,(qmc.Sobol(2,scramble=True,seed=29004).random_base2(15)-.5)*SPAN,peak_samples(14,29452,1500)));zextra=source.sample(extra);valid=np.isfinite(zextra);q=np.vstack((native,extra[valid]));truth=np.r_[znative,zextra[valid]]
 print(json.dumps(dict(stage='fit-operator',controls=len(xy),samples=len(q),demTiles=len(source.manifest))),flush=True);a=__import__('free_mls').operator(xy,q,support,curvature=1e-5,tree=support.tree);z=np.load(out/'z.npy') if args.resume_fit else source.sample(xy);limits=np.where(np.linalg.norm(q,axis=1)<=2000,6.25,12.5);weights=1/limits;best=(float(np.max(abs(a@z-truth)/limits)),z.copy());started=time.perf_counter()
 for it in range(0 if args.resume_fit else 6):
  result=lsmr(a.multiply(weights[:,None]),(truth-a@z)*weights,damp=.003,maxiter=500,atol=1e-7,btol=1e-7);z+=result[0];err=a@z-truth;score=float(np.max(abs(err)/limits))
  if score<best[0]:best=(score,z.copy());np.save(out/'z.npy',z)
  print(json.dumps(dict(stage='fit',iteration=it,normalizedError=score,solverIterations=int(result[2]),seconds=time.perf_counter()-started)),flush=True)
  if score<.8:break
  weights=np.maximum(weights,np.clip(abs(err)/limits,1,24)**2/limits)
 z=best[1];np.save(out/'z.npy',z)
 v=np.vstack(((qmc.Sobol(2,scramble=True,seed=29904).random_base2(15)-.5)*SPAN,peak_samples(14,29935,1000)));vt=source.sample(v);mask=np.isfinite(vt);v=v[mask];vt=vt[mask];av=__import__('free_mls').operator(xy,v,support,curvature=1e-5,tree=support.tree);trials=[];chosen=None
 for step in (.125,.5,1,2,4):
  blob=encode_quant(np.column_stack((xy,z)),xy=.0625,zstep=step);p=decode_quant(blob);np.testing.assert_array_equal(p[:,:2],xy);audit=requirements(a@p[:,2]-truth,q,12.5);validation=requirements(av@p[:,2]-vt,v,12.5);score=max(audit['measuredMaxError']/12.5,audit['measuredPeakMaxError']/6.25,validation['measuredMaxError']/12.5,validation['measuredPeakMaxError']/6.25);item=dict(step=step,bytes=len(blob),audit=audit,validation=validation,score=score);trials.append(item);passed=audit['converged'] and validation['converged'];key=(not passed,len(blob) if passed else score)
  if chosen is None or key<chosen[0]:chosen=(key,item,blob,p)
 _,item,blob,p=chosen;(out/'model.mlq1').write_bytes(blob);(out/'trials.json').write_text(json.dumps(trials,indent=2),encoding='utf-8');del a,av;gc.collect()
 info=dict(targetError=12.5,controls=len(p),reconstruction='independent-MLS',reference='original-PNG-DEM',center=source.center.tolist(),sourcePixelStepProjectedMeters=STEP,sourceManifestSha256=source.digest,**item);(out/'provisional.json').write_text(json.dumps(info,indent=2),encoding='utf-8');print('MODEL_READY',json.dumps(info),flush=True)
 metadata=dict(region='yari',span=SPAN,format='MLQ1',parent=None,support='direct-k24-scale1.5-floor2',curvaturePenalty=1e-5,reference='original-PNG-DEM',sourceManifestSha256=source.digest,distributable=False);(out/'metadata.json').write_text(json.dumps(metadata,indent=2),encoding='utf-8')
 # 全元画素。欠損は0に置き換えず、除外数を記録する。
 maximum=0.;peakmax=0.;squared=0.;count=0;missing=0;worst=None;verify_started=time.perf_counter()
 for block,(points,truth,invalid) in enumerate(source.native_rows(16)):
  actual=evaluate(xy,p[:,2],points,support,chunk=8192);error=actual-truth;i=int(np.argmax(abs(error)));m=float(abs(error[i]))
  if m>maximum:maximum=m;worst=dict(xy=points[i].tolist(),dem=float(truth[i]),model=float(actual[i]))
  peak=np.linalg.norm(points,axis=1)<=2000
  if peak.any():peakmax=max(peakmax,float(abs(error[peak]).max()))
  squared+=float(error@error);count+=len(error);missing+=invalid
  if block%8==0:
   progress=dict(stage='native-DEM-verification',samples=count,maxError=maximum,rmse=(squared/count)**.5,seconds=time.perf_counter()-verify_started);(out/'progress.json').write_text(json.dumps(progress,indent=2),encoding='utf-8');print(json.dumps(progress),flush=True)
 native_measure=dict(measuredMaxError=maximum,measuredPeakMaxError=peakmax,measuredRmse=(squared/count)**.5,converged=maximum<=12.5 and peakmax<=6.25,samples=count,missing=missing,worst=worst);info.update(nativeDemValidation=native_measure);(out/'info.json').write_text(json.dumps(info,indent=2),encoding='utf-8');print('RESULT',json.dumps(info),flush=True)
 # 元DEMとモデルを、全域および山頂寄り8kmで比較する描画データ。
 view=Path('build/mls-generated-preview/yari-dem');view.mkdir(exist_ok=True);n=1024
 for span in (32000,8000):
  axis=np.linspace(-span/2,span/2,n+1);x,y=np.meshgrid(axis,axis);g=np.column_stack((x.ravel(),y.ravel()));dem=source.sample(g).reshape(n+1,n+1);model=evaluate(xy,p[:,2],g,support).reshape(n+1,n+1)
  for label,h in [('dem',dem),('model',model)]:
   gy,gx=np.gradient(h,span/n,edge_order=2);np.stack((h,gx,gy),axis=2).astype('<f4').tofile(view/f'{label}-{span}.bin')
 (view/'info.json').write_text(json.dumps(dict(n=n,span=SPAN,**info,**native_measure),indent=2),encoding='utf-8');print('PREVIEW_READY',flush=True)
if __name__=='__main__':main()
