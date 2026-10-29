"""全品質・全タイルの誤差測定に基づく制御点削減。再実行で続行可能。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json,time,sqlite3,shutil,hashlib,ctypes,traceback
from pathlib import Path
import numpy as np
from scipy.stats import qmc
import natural_c1_128 as worker
from audit_reduced_c1_128 import dem
from test_fixed_lpf_regions import ROOT,write
from sibson_global_c1 import GlobalC1
from weighted_sibson import WeightedSibson
from sibson_aux_surface import AuxiliarySibsonSurface
BASE=ROOT/'build/natural-raw-c1-128-v1'
OUT=ROOT/'build/natural-raw-c1-128-reduced-all-v1'
def evaluate(folder,q,reference,error):
 data=np.load(folder/'control-gradient.npz');p=data['points'];model=GlobalC1.__new__(GlobalC1);model.points=p;model.model=WeightedSibson(p);model.gradient=np.ascontiguousarray(data['gradient']);model.fast_model=AuxiliarySibsonSurface(p)
 model.fast_lib=ctypes.CDLL(str(ROOT/'build/sibson_global_c1_eval.dll'));d=np.ctypeslib.ndpointer(dtype=np.float64,flags='C_CONTIGUOUS');i=np.ctypeslib.ndpointer(dtype=np.int32,flags='C_CONTIGUOUS');model.fast_lib.sibson_aux_c1.argtypes=[d,d,d,i,i,d,i,i,i,d,d,i,ctypes.c_int,d,i];model.fast_lib.sibson_aux_c1.restype=ctypes.c_int
 valid=np.isfinite(reference);a=model.model(q)[valid];b=model(q)[valid];ref=reference[valid]
 if not len(ref):return dict(samples=0,controls=len(p)-4,max=0,rmse=0,passes=True)
 e=np.maximum(abs(a-ref),abs(b-ref));return dict(samples=len(ref),controls=len(p)-4,max=float(e.max()),rmse=float(np.sqrt(np.mean(e*e))),passes=bool(e.max()<=error*.9))
def build_one(region,error,x,y):
 original=BASE/region/f'{error:g}'/f'{x}-{y}';target=OUT/region/f'{error:g}'/f'{x}-{y}';target.mkdir(parents=True,exist_ok=True);meta=json.loads((original/'packet.json').read_text());side=meta['side']
 if meta.get('empty'):
  write(target/'packet.json',meta);write(target/'selection.json',dict(empty=True));return
 origin=np.asarray(json.loads((BASE/region/'source/region.json').read_text())['origin']);q=qmc.Sobol(2,scramble=True,seed=55219).random_base2(16)*side+[x*side,y*side];ref=dem(q+origin);local=q-np.asarray(meta['origin']);reports=[];selected=None
 for variant,factor in [('eighth',.125),('quarter',.25),('half',.5),('sibson',1)]:
  folder=(BASE if variant=='sibson' else ROOT/f'build/natural-raw-c1-128-{variant}-v1')/region/f'{error:g}'/f'{x}-{y}'
  if not (folder/'packet.json').exists():
   worker.RUN=ROOT/f'build/natural-raw-c1-128-{variant}-v1';worker.RUN.mkdir(exist_ok=True);worker.SOURCE_RUN=BASE;worker.BUDGET_FACTOR=factor;worker.build(region,error,x,y)
  result=evaluate(folder,local,ref,error);result['variant']=variant;reports.append(result)
  if result['passes'] or variant=='sibson':selected=folder;break
 for name in ['packet.bin','control-gradient.npz']:
  shutil.copyfile(selected/name,target/name)
 info=json.loads((selected/'packet.json').read_text());info['reductionAudit']=dict(sampledOnly=True,threshold=error*.9,reports=reports,selected=reports[-1]['variant']);write(target/'packet.json',info);write(target/'selection.json',info['reductionAudit'])
def run():
 OUT.mkdir(exist_ok=True)
 lock=sqlite3.connect(OUT/'run-lock.sqlite3',timeout=0);lock.execute('BEGIN EXCLUSIVE')
 release=ROOT/'build/natural-fixed-lpf-target/release/terrain-natural-national.exe';tag=hashlib.sha256(release.read_bytes()).hexdigest()[:16];worker.ENGINE=OUT/f'engine-{tag}.exe'
 if not worker.ENGINE.exists():shutil.copyfile(release,worker.ENGINE)
 started=time.monotonic();failed={};done=0
 while True:
  with sqlite3.connect(BASE/'queue.sqlite3') as c:rows=c.execute("SELECT region,error,x,y,state FROM jobs").fetchall()
  def priority(r):
   region,error,x,y,_=r;n=round(128000/dict(worker.LEVELS)[error]);central=abs(x+.5-n/2)<=1 and abs(y+.5-n/2)<=1
   return (not central,-error,region,y,x)
  rows.sort(key=priority);pending=[];done=0
  for region,error,x,y,state in rows:
   key=f'{region}/{error:g}/{x}-{y}'
   if (OUT/key/'selection.json').exists():done+=1;continue
   if state=='ready' and key not in failed:pending.append((region,error,x,y))
  write(OUT/'progress.json',dict(complete=done==len(rows),done=done,total=len(rows),readyPending=len(pending),failed=failed,seconds=time.monotonic()-started))
  if done==len(rows):return
  if not pending:time.sleep(10);continue
  region,error,x,y=pending[0];key=f'{region}/{error:g}/{x}-{y}'
  write(OUT/'active.json',dict(tile=key,started=time.time()))
  try:build_one(region,error,x,y);print('ready',key,flush=True)
  except Exception:failed[key]=traceback.format_exc();print(failed[key],flush=True)
if __name__=='__main__':run()
