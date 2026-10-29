"""128km制御点削減版を元DEMの独立サンプルで比較する（上限保証ではない）。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json,time,shutil
import numpy as np
from scipy.stats import qmc
from PIL import Image
from test_fixed_lpf_regions import ROOT,STEP,write
from sibson_global_c1 import GlobalC1
BASE=ROOT/'build/natural-raw-c1-128-v1'
def dem(q):
 pixel=np.floor(q/STEP).astype(np.int64);tile=pixel//256;out=np.full(len(q),np.nan)
 keys,groups=np.unique(tile,axis=0,return_inverse=True)
 for i,(x,y) in enumerate(keys):
  path=ROOT/f'build/terrain-cache/14/{x}/{y}.png'
  if not path.exists():continue
  ids=np.flatnonzero(groups==i);rgb=np.asarray(Image.open(path)).astype(np.int32);v=(rgb[:,:,0]<<16)|(rgb[:,:,1]<<8)|rgb[:,:,2];a=v[pixel[ids,1]%256,pixel[ids,0]%256];out[ids]=np.where(a>=8388608,a-16777216,a)*.01;out[ids[a==8388608]]=np.nan
 return out

def run():
 reports=[];started=time.monotonic()
 for region in ['fuji','yari','tsukuba']:
  origin=np.asarray(json.loads((BASE/region/'source/region.json').read_text())['origin']);q=qmc.Sobol(2,scramble=True,seed=55219).random_base2(16)*128000;reference=dem(q+origin);valid=np.isfinite(reference);local=q-64000
  for variant in ['sibson','half','quarter','eighth']:
   folder=(BASE if variant=='sibson' else ROOT/f'build/natural-raw-c1-128-{variant}-v1')/region/'800/0-0';data=np.load(folder/'control-gradient.npz');points=data['points'];model=GlobalC1(points,queries=local[:4096]);model.gradient=np.ascontiguousarray(data['gradient']);c1=model(local);plain=model.model(local);mix=plain*.55+c1*.45
   row=dict(region=region,variant=variant,controls=len(points)-4,samples=int(valid.sum()),sampledOnly=True)
   for name,a in [('sibson',plain),('c1',c1),('mix45',mix)]:
    e=np.abs(a[valid]-reference[valid]);row[name]=dict(max=float(e.max()),p99=float(np.quantile(e,.99)),rmse=float(np.sqrt(np.mean(e*e))),over800=int((e>800).sum()))
   reports.append(row);write(BASE/'reduction-audit.json',dict(complete=False,reports=reports,seconds=time.monotonic()-started));print(json.dumps(row),flush=True)
 write(BASE/'reduction-audit.json',dict(complete=True,reports=reports,seconds=time.monotonic()-started))
def select_reduced():
 audit=json.loads((BASE/'reduction-audit.json').read_text());chosen=[];root=ROOT/'build/natural-raw-c1-128-reduced-v1'
 if not audit['complete']:raise ValueError('誤差測定が未完了です')
 for region in ['fuji','yari','tsukuba']:
  rows=[r for r in audit['reports'] if r['region']==region and max(r['sibson']['max'],r['c1']['max'])<=720]
  if not rows:raise ValueError(f'{region}: 閾値以下の方式がありません')
  row=min(rows,key=lambda r:r['controls']);variant=row['variant'];src=(BASE if variant=='sibson' else ROOT/f'build/natural-raw-c1-128-{variant}-v1')/region/'800/0-0';dst=root/region/'800/0-0';dst.mkdir(parents=True,exist_ok=True)
  for name in ['packet.bin','packet.json','control-gradient.npz']:shutil.copyfile(src/name,dst/name)
  chosen.append(row)
 write(root/'selection.json',dict(sampledOnly=True,threshold=720,chosen=chosen))
if __name__=='__main__':
 run();select_reduced()
