"""完成した全域版のXYZ独立Rice圧縮とGPU容量を実測。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
from concurrent.futures import ProcessPoolExecutor
from functools import lru_cache
import json,sqlite3,time
from pathlib import Path
import numpy as np
from natural_c1_128 import ROOT,RUN as BASE
from test_fixed_lpf_regions import write
from sibson_native_codec import encode_independent,decode_independent
OUT=ROOT/'build/natural-raw-c1-128-errorfit-full-v1'
@lru_cache(maxsize=3)
def origin(region):return np.asarray(json.loads((BASE/region/'source/region.json').read_text())['origin'])
def measure(row):
 region,error,x,y,raw=row;a=json.loads(raw);folder=OUT/region/f'{error:g}'/f'{x}-{y}';r=dict(region=region,error=error,points=a.get('finalPoints',0),empty=bool(a.get('empty')),riceBytes=0,gpuBytes=(folder/'packet.bin').stat().st_size if (folder/'packet.bin').exists() else 0)
 if r['empty']:return r
 cached=folder/'xyz-rice-size.json'
 if cached.exists():r['riceBytes']=json.loads(cached.read_text())['riceBytes'];return r
 info=json.loads((folder/'packet.json').read_text())
 with np.load(folder/'control-gradient.npz') as data:p=data['points']
 world=origin(region)+np.asarray(info['origin']);oq=np.rint(world/.0625).astype(np.int64);q=np.empty(p.shape,np.int64);q[:,:2]=np.rint((p[:,:2]+world)/.0625).astype(np.int64)-oq;q[:,2]=np.rint(p[:,2]/.125).astype(np.int64);blob=encode_independent(q);assert np.array_equal(q,decode_independent(blob));(folder/'control-xyz.nqc1').write_bytes(blob);r['riceBytes']=len(blob);write(cached,dict(riceBytes=len(blob),xyzOriginQ=oq.tolist(),codec='NQC1 Rice/FOR/zero-run independent',steps=[.0625,.0625,.125],roundtripVerified=True,runtimeUsesThisBlob=False));return r

def run():
 start=time.monotonic()
 with sqlite3.connect(OUT/'queue.sqlite3') as c:rows=c.execute("SELECT region,error,x,y,info FROM jobs WHERE state='ready'").fetchall()
 result=[]
 with ProcessPoolExecutor(max_workers=max(1,(os.cpu_count() or 2)//2)) as pool:
  for i,r in enumerate(pool.map(measure,rows,chunksize=16)):
   result.append(r)
   if i%256==0:write(OUT/'size-progress.json',dict(done=i+1,total=len(rows),seconds=time.monotonic()-start))
 levels=[]
 for e in [800,400,200,100,50,25,12.5]:
  a=[r for r in result if r['error']==e];level=dict(error=e,sideKm=128*e/800,tiles=len(a))
  for k in ['points','empty','riceBytes','gpuBytes']:level[k]=sum(r[k] for r in a)
  levels.append(level);print(json.dumps(level),flush=True)
 write(OUT/'size-report-rice-complete.json',dict(complete=True,tiles=len(rows),levels=levels,seconds=time.monotonic()-start,note='XYZ independent NQC1 Rice only; all integer roundtrips verified; includes 4 guard points in compressed data, excludes them in displayed counts; no level delta. GPU packets include gradients/Voronoi/index/metadata but not request validityMask, HTTP, SQLite or work/source caches.'))
if __name__=='__main__':run()
