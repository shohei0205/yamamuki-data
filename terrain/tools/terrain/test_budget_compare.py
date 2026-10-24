"""20%余裕枠の比較を検証し、山頂周辺の誤差を追加計測する。"""
import json,sqlite3
import numpy as np
from pathlib import Path
from test_overlap_expanded import raster
from test_seam_compare import validate
ROOT=Path(__file__).resolve().parents[2];OUT=ROOT/'build/additional-budget-20261024'
def run():
 with sqlite3.connect(OUT/'comparison.sqlite3') as db:items=db.execute('select region,method,error,preview,report from meshes order by region,method,error').fetchall()
 assert len(items)==84
 heights=[];previous={};axis=np.arange(0,16001,40);x,y=np.meshgrid(axis,axis);peak=(x-8000)**2+(y-8000)**2<=1000**2
 for region,method,error,raw,report in items:
  mesh=json.loads(raw);r=json.loads(report);validate(mesh,0)
  assert r['maximum']<=error+1e-6 and r['minimumAngle']>=10-1e-6
  if method=='summit-error':assert r['localConstraintMaximum']<=error+1e-6
  points={tuple(p) for p in mesh['points']};key=(region,method)
  if key in previous:assert points<=previous[key]
  previous[key]=points
  dem=np.fromfile(ROOT/f'build/decimation-compare-20261023/{region}.f64',dtype='<f8').reshape(1601,1601)[::4,::4];z=raster(mesh);diff=z-dem
  r.update(region=region,method=method,sampleRmse=float(np.sqrt(np.mean(diff**2))),summitRmse=float(np.sqrt(np.mean(diff[peak]**2))),summitMaximum=float(np.max(np.abs(diff[peak]))),summitHeightError=float(np.max(z[peak])-np.max(dem[peak])))
  heights.append(r)
  print(region,method,error,'検証済み',flush=True)
 with sqlite3.connect(OUT/'comparison.sqlite3') as db:
  for r in heights:db.execute('update meshes set report=? where region=? and method=? and error=?',(json.dumps({k:v for k,v in r.items() if k not in ('region','method')}),r['region'],r['method'],r['error']))
 (OUT/'audit.json').write_text(json.dumps(heights,ensure_ascii=False,indent=2),encoding='utf-8')
 print('84メッシュの接続・誤差・角度・頂点包含と山頂誤差を確認')
if __name__=='__main__':run()
