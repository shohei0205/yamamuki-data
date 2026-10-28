"""診断時だけGPUから読んだ値を、CPUの元曲面と比較する。通常描画は読まない。"""
import json
from pathlib import Path
import numpy as np
from sibson_aux_surface import AuxiliarySibsonSurface
from sibson_c1 import SibsonC1
ROOT=Path('build/sibson-weighted-v1');reports=[];models={}
for path in sorted((ROOT/'gpu-controls').glob('*-samples.json')):
 case=json.loads(path.read_text());region,error,n=case['region'],case['error'],case['n'];key=(region,error)
 if key not in models:
  with np.load(ROOT/f'{region}-{error}'/'natural-global-c1.cache.npz') as saved:points=saved['points'].copy();gradient=saved['gradient'].copy()
  models[key]=(AuxiliarySibsonSurface(points),SibsonC1(points,gradient=gradient))
 values=np.asarray(case['values']).reshape(-1,8);index=np.asarray(case['indices']);q=np.column_stack((-4000+(index%(n+1))*8000/n,-4000+(index//(n+1))*8000/n));result=dict(region=region,error=error,n=n,samples=len(q),methods=[])
 for offset,model in zip([0,4],models[key]):
  expected=model(q);height=values[:,offset]-expected;grad=[]
  for j,delta in enumerate([np.array([1.,0]),np.array([0,1.])]):grad.append(values[:,offset+j+1]-(model(q+delta)-model(q-delta))/2)
  hmax=float(abs(height).max());gmax=float(np.max(abs(np.asarray(grad))));assert hmax<.01,(path,hmax);assert gmax<.005,(path,gmax);result['methods'].append(dict(maximumHeightDifference=hmax,heightRmse=float(np.sqrt(np.mean(height*height))),maximumGradientDifference=gmax))
 reports.append(result);print(region,error,n,result['methods'],flush=True)
(ROOT/'gpu-controls'/'verification.json').write_text(json.dumps(reports,indent=2),encoding='utf-8')
