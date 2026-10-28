"""高点数比較モデルを、未知数の16倍以上の訓練点で再適合する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json
from pathlib import Path
import numpy as np
from scipy.stats import qmc
from mls_lpf_dem import FilteredDem
from raw_dem_reference import RawDem,center_for
from weighted_sibson import fit
from free_rational_bspline import FreeRationalSpline,encode,decode
ROOT=Path('build/sibson-weighted-v1')
for region in ['fuji','tsukuba','yari']:
 for error in [50,25]:
  folder=ROOT/f'{region}-{error}';name='free-bspline-dense';path=folder/(name+'.frb1');model=decode(path.read_bytes());report=json.loads((folder/'report.json').read_text());filtered=FilteredDem(region,report['sigmaMeters']);raw=RawDem('build/terrain-cache',center_for(region));power=max(15,int(np.ceil(np.log2(len(model.points)*16))));q=(qmc.Sobol(2,scramble=True,seed=1031).random_base2(power)-.5)*16000;h=filtered.sample(q);valid=np.isfinite(h);q=q[valid];h=h[valid];a=model.basis(q);z,w,history=fit(a,h,filtered.sample(model.points[:,:2]),error,iterations=8,z_regular=.0003);model=FreeRationalSpline(np.column_stack((model.points[:,:2],z)),model.scales,model.angles,w);blob=encode(model);model=decode(blob);path.write_bytes(blob)
  held=(qmc.Sobol(2,scramble=True,seed=987).random_base2(15)-.5)*15990;truth=raw.sample(held);valid=np.isfinite(truth);e=model(held[valid])-truth[valid];axis=np.linspace(-8000,8000,513);x,y=np.meshgrid(axis,axis);field=model(np.column_stack((x.ravel(),y.ravel()))).reshape(513,513);gy,gx=np.gradient(field,axis,axis);np.stack((field,gx,gy),axis=-1).astype('<f4').tofile(folder/(name+'.field'));controls=np.column_stack((model.points,model.weights,model.scales,model.angles,model(model.points[:,:2])));(folder/(name+'.controls.json')).write_text(json.dumps(controls.tolist()),encoding='utf-8');item=report['variants'][name];item.update(bytes=len(blob),rawRmse=float(np.sqrt(np.mean(e*e))),rawMaxError=float(abs(e).max()),trainingSamples=len(q));report['freeSpline']['denseRefit']=dict(history=history,samples=len(q));(folder/'report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8');print(region,error,item,flush=True)
