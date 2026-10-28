"""3山の中心16 kmで、重みと同容量の点追加を比較する。既存データは変更しない。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json,sqlite3,time,zlib,sys
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree
from scipy.stats import qmc
from weighted_sibson import WeightedSibson,fit,encode_weighted,decode_weighted
from sibson_surface import SibsonSurface
from sibson_variable128 import CircleIndex
from mls_quant_codec import encode_quant,decode_quant
from raw_dem_reference import RawDem,center_for
from mls_lpf_dem import FilteredDem
ROOT=Path('build/sibson-weighted-v1');ROOT.mkdir(exist_ok=True)
def write(p,value):p.write_text(json.dumps(value,ensure_ascii=False,indent=2),encoding='utf-8')
def run(region,error):
 start=time.perf_counter();folder=ROOT/f'{region}-{error:g}';folder.mkdir(exist_ok=True)
 with sqlite3.connect(f'build/sibson-region128-v1/{region}/terrain.sqlite3') as db:blob,info=db.execute('select blob,info from models where error=?',(error,)).fetchone()
 info=json.loads(info);global_model=SibsonSurface(decode_quant(blob));p=CircleIndex(global_model).extract([55800,55800,72200,72200]).astype(float);p[:,:2]-=64000
 # 表示16 kmの周囲にも元の点を残して、切り出しの人工境界を避ける。
 m=WeightedSibson(p);rng=np.random.default_rng(340);probe=rng.uniform(-7900,7900,(1024,2));difference=float(abs(m(probe)-global_model(probe+64000)).max());del global_model
 if difference>1e-5:raise ValueError(f'切り出しで曲面が変化: {difference}')
 filtered=FilteredDem(region,info['sigmaMeters']);raw=RawDem('build/terrain-cache',center_for(region));q=(qmc.Sobol(2,scramble=True,seed=31).random_base2(15)-.5)*16000
 control=p[np.all(abs(p[:,:2])<7999,axis=1),:2];q=np.vstack((q,control));truth=filtered.sample(q);valid=np.isfinite(truth);q=q[valid];truth=truth[valid]
 a=m.basis(q);z,w,history=fit(a,truth,p[:,2],error);zonly,_,_=fit(a,truth,p[:,2],error,use_weights=False)
 # 重みはlog2を1/32刻みの1 byte、XYZは既存MLQ1と同じ刻みで保存。
 code=np.rint((np.log2(w)+2)*32).clip(0,128).astype('u1');wb=zlib.compress(code.tobytes(),6);qw=2**(code.astype(float)/32-2)
 variants={}
 def save(name,points,weights=None,weightblob=b''):
  if weights is None:
   b=encode_quant(points,xy=.0625,zstep=.125);points=decode_quant(b).astype(float);weights=np.ones(len(points))
  else:
   b,weightblob=encode_weighted(points,weights);points,weights=decode_weighted(b,weightblob)
  (folder/(name+'.mlq1')).write_bytes(b)
  if weightblob:(folder/(name+'.weights')).write_bytes(weightblob)
  variants[name]=dict(points=points,weights=weights,bytes=len(b)+len(weightblob),geometryBytes=len(b),weightBytes=len(weightblob))
 save('original',p);save('zfit',np.column_stack((p[:,:2],zonly)));save('weighted',np.column_stack((p[:,:2],z)),qw,wb)
 budget=variants['weighted']['bytes'];residual=abs(a@p[:,2]-truth);order=np.argsort(-residual,kind='stable');tree=cKDTree(p[:,:2]);dist,_=tree.query(q);chosen=[];occupied=set()
 for ix in order:
  if dist[ix]<16:continue
  cell=tuple(np.floor(q[ix]/32).astype(int))
  if cell in occupied:continue
  occupied.add(cell);chosen.append(ix)
  if len(chosen)>=max(128,len(p)//2):break
 extra=np.column_stack((q[chosen],truth[chosen]));lo=0;hi=len(extra)
 while lo<hi:
  mid=(lo+hi+1)//2;b=encode_quant(np.vstack((p,extra[:mid])),xy=.0625,zstep=.125)
  if len(b)<=budget:lo=mid
  else:hi=mid-1
 save('extra',np.vstack((p,extra[:lo])))
 # 点追加案にも同じZ再適合を行い、重み案だけに最適化の利点を与えない。
 for retry in range(8):
  ep=np.vstack((p,extra[:lo]));em=WeightedSibson(ep);ez,_,_=fit(em.basis(q),truth,ep[:,2],error,use_weights=False);ep[:,2]=ez
  eb=encode_quant(ep,xy=.0625,zstep=.125)
  if len(eb)<=budget or lo==0:break
  lo=max(0,lo-max(1,int((len(eb)-budget)/5)+1))
 save('extra-zfit',ep)
 if variants['extra-zfit']['bytes']>budget:raise ValueError('点追加の容量が予算を超えました')
 held=(qmc.Sobol(2,scramble=True,seed=987).random_base2(15)-.5)*15990;truthraw=raw.sample(held);target=filtered.sample(held);valid=np.isfinite(truthraw)&np.isfinite(target);held=held[valid];truthraw=truthraw[valid];target=target[valid];report={}
 axis=np.linspace(-8000,8000,513);xx,yy=np.meshgrid(axis,axis);grid=np.column_stack((xx.ravel(),yy.ravel()));valid_grid=np.isfinite(raw.sample(grid));truth_grid=raw.sample(grid)
 for name,v in variants.items():
  model=WeightedSibson(v['points'],v['weights']);prediction=model(held);h=model(grid).reshape(513,513);gy,gx=np.gradient(h,axis,axis);field=np.stack((h,gx,gy),axis=-1).astype('<f4');(folder/(name+'.field')).write_bytes(field.tobytes())
  e=prediction-truthraw;et=prediction-target
  report[name]=dict(controls=len(v['points']),bytes=v['bytes'],geometryBytes=v['geometryBytes'],weightBytes=v['weightBytes'],rawMaxError=float(abs(e).max()),rawRmse=float(np.sqrt(np.mean(e*e))),filteredRmse=float(np.sqrt(np.mean(et*et))),sampleCount=len(held),weightMinimum=float(v['weights'].min()),weightMaximum=float(v['weights'].max()))
 h=truth_grid.reshape(513,513);gy,gx=np.gradient(h,axis,axis);np.stack((h,gx,gy),axis=-1).astype('<f4').tofile(folder/'dem.field')
 report=dict(region=region,error=error,span=16000,n=512,sigmaMeters=info['sigmaMeters'],variants=report,cutoutMaximumDifference=difference,history=history,seconds=time.perf_counter()-start,reference='元PNG DEMの独立Sobol32768点。全画素の最大誤差保証ではない。',xyFixed=True,weightFormula='sum(lambda_i*w_i*z_i)/sum(lambda_i*w_i)',quantization=dict(xy=.0625,z=.125,weightLog2Step=1/32))
 write(folder/'report.json',report);print(json.dumps(report,ensure_ascii=False),flush=True)
if __name__=='__main__':
 for region in (sys.argv[1:] or ['fuji','tsukuba','yari']):
  for error in [50.,25.]:
   write(ROOT/'progress.json',dict(region=region,error=error,stage='fitting'));run(region,error)
 write(ROOT/'progress.json',dict(stage='complete'))
