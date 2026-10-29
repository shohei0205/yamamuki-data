"""12.5m・山周辺8kmで、XY不変の元DEM連続復元比較を作る。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json,time
from pathlib import Path
import numpy as np
from scipy.stats import qmc
from continuous_dem import sample,STEP
from natural_c1_128 import ROOT,RUN as BASE,write,GlobalC1,close_cells,pack
OUT=ROOT/'build/natural-raw-c1-128-continuous-v1'
DEM_METADATA={}
def build(region,x,y):
 src=BASE/region/'12.5'/f'{x}-{y}';dst=OUT/region/'12.5'/f'{x}-{y}';dst.mkdir(parents=True,exist_ok=True)
 if (dst/'selection.json').exists():return
 info=json.loads((src/'packet.json').read_text())
 if info.get('empty'):write(dst/'packet.json',info);write(dst/'selection.json',dict(empty=True));return
 data=np.load(src/'control-gradient.npz');points=data['points'].copy();original=points.copy();r=json.loads((BASE/region/'source/region.json').read_text());world=np.asarray(r['origin'])+np.asarray(info['origin']);halo=info['halo'];side=info['side'];real=np.all(np.abs(points[:,:2])<=side/2+halo+1,axis=1)
 heights=sample(points[real,:2]+world);valid=np.isfinite(heights);ids=np.flatnonzero(real)[valid];points[ids,2]=np.round(heights[valid]/.125)*.125
 bounds=np.asarray(info['bounds']);n=max(16384,min(262144,2**int(np.ceil(np.log2(len(points)*4)))));queries=qmc.Sobol(2,scramble=True,seed=9124).random_base2(int(np.log2(n)))*(bounds[2:]-bounds[:2])+bounds[:2]
 model=GlobalC1(points,strength=.01,queries=queries);close_cells(model.fast_model)
 metadata={k:info[k] for k in ['region','error','x','y','side','origin','coreBounds','worldFieldBounds','halo']};metadata.update(curve='Sibson-global-C1-continuous-DEM',lpf=False,sourceGeometryBytes=len(points)*12,maxErrorGuaranteed=False,sourceInterpolation='symmetric-shape-preserving-cubic',sameXY=True)
 metadata.update(DEM_METADATA)
 packet,out=pack(points,model.gradient,bounds,metadata,model=model.fast_model,max_candidates=32,max_depth=8)
 if out['unsafeLeaves']:raise ValueError('unsafe Voronoi cells')
 # 独立の画素中心に対する事後誤差。元DEM画素を補間せず直接参照。
 from audit_reduced_c1_128 import dem
 q=qmc.Sobol(2,scramble=True,seed=65428).random_base2(14)*side-side/2;w=q+world;w=(np.floor(w/STEP)+.5)*STEP;q=w-world;reference=dem(w);a=model.model(q);b=model(q);ok=np.isfinite(reference);e=np.maximum(abs(a[ok]-reference[ok]),abs(b[ok]-reference[ok]));audit=dict(samples=int(ok.sum()),max=float(e.max()) if len(e) else 0,rmse=float(np.sqrt(np.mean(e*e))) if len(e) else 0,pointZChangeMax=float(np.abs(points[:,2]-original[:,2]).max()),sampledOnly=True)
 (dst/'packet.bin').write_bytes(packet);np.savez_compressed(dst/'control-gradient.npz',points=points,gradient=model.gradient);out['continuousAudit']=audit;write(dst/'packet.json',out);write(dst/'selection.json',audit)
 print(region,x,y,audit,flush=True)
def run():
 OUT.mkdir(exist_ok=True);done=[];started=time.monotonic()
 for region in ['fuji','yari','tsukuba']:
  for y in range(30,34):
   for x in range(30,34):
    build(region,x,y);done.append([region,x,y]);write(OUT/'progress.json',dict(done=len(done),total=48,complete=len(done)==48,seconds=time.monotonic()-started))
if __name__=='__main__':run()
