"""実GPUから試験時だけ読み戻した標高を、独立した倍精度評価と比較する。"""
import json
from pathlib import Path
import numpy as np
from sibson_gpu_variable import packet
from test_sibson_gpu_variable import evaluate

root=Path('build/sibson-gpu-variable-v1');reports=[]
for path in sorted(root.glob('*-samples.json')):
 rows=json.loads(path.read_text());maximum=0.;count=0
 for row in rows:
  t=row['tile'];info=row['info'];n=t['n'];ids=np.asarray(row['indices']);bounds=np.asarray(info['bounds']);fraction=np.c_[ids%(n+1),ids//(n+1)]/n;q=bounds[:2]+fraction*(bounds[2:]-bounds[:2]);expected=evaluate(packet(info['region'],t['error'],t['x'],t['y']),q);actual=np.asarray(row['values'])[:,0];difference=float(abs(expected-actual).max());maximum=max(maximum,difference);count+=len(ids)
  if row['invalid'] or difference>.02:raise ValueError((path,t,difference,row['invalid']))
 reports.append(dict(case=path.stem,points=count,maximumHeightDifference=maximum));print(path.stem,count,maximum,flush=True)
if len(reports)!=21:raise ValueError('3地域・7レベルのGPU結果が揃っていません')
(root/'gpu-verification.json').write_text(json.dumps(reports,indent=2),encoding='utf-8')
