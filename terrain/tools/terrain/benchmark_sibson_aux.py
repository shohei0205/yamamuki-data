"""実データで旧Sibsonとの一致・評価速度・補助情報量を測定する。"""
import json,time,zlib
from pathlib import Path
import numpy as np
from sibson_region128_server import database,restore
from sibson_surface import SibsonSurface
from sibson_aux_surface import AuxiliarySibsonSurface
root=Path('build/sibson-aux-comparison-v1');root.mkdir(parents=True,exist_ok=True);reports=[]
for region in ['fuji','yari','tsukuba']:
 for error in [400,100,25]:
  with database(region) as db:
   row=db.execute('select x,y,error from tiles order by abs(x-8)+abs(y-8),abs(error-?) limit 1',(error,)).fetchone();p=restore(db,*row)
  axisx=np.linspace(row[0]*8000+.1,(row[0]+1)*8000-.1,128);axisy=np.linspace(row[1]*8000+.1,(row[1]+1)*8000-.1,128);x,y=np.meshgrid(axisx,axisy);q=np.column_stack((x.ravel(),y.ravel()))
  start=time.perf_counter();a=SibsonSurface(p);baseline_build=time.perf_counter()-start;b=AuxiliarySibsonSurface(p);times=[];outputs=[]
  for model in [a,b]:
   model(q[:100]);elapsed=[]
   for _ in range(5):
    start=time.perf_counter();h=model(q);elapsed.append(time.perf_counter()-start)
   times.append(float(np.median(elapsed)));outputs.append(h)
  difference=float(abs(outputs[0]-outputs[1]).max());np.testing.assert_allclose(outputs[0],outputs[1],atol=1e-5,rtol=1e-10)
  blob=b.auxiliary_blob();c=AuxiliarySibsonSurface(p,blob=blob);np.testing.assert_array_equal(c(q),outputs[1]);(root/f'{region}-{error}.sau').write_bytes(blob)
  r=dict(region=region,error=error,points=len(p),queries=len(q),baselineBuildSeconds=baseline_build,auxBuildSeconds=b.aux_build_seconds,baselineSeconds=times[0],auxSeconds=times[1],speedup=times[0]/times[1],maximumDifferenceMeters=difference,auxiliaryBytes=len(blob),auxiliaryRamBytes=b.aux_bytes,xyzFloat32ZlibBytes=len(zlib.compress(p.astype('<f4').tobytes(),6)),stats=b.last_chunk_stats);reports.append(r);print(json.dumps(r),flush=True)
(root/'benchmark.json').write_text(json.dumps(reports,indent=2),encoding='utf-8')
