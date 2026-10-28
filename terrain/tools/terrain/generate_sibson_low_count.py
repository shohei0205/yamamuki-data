"""点数を 128 点まで減らした自然近傍曲面を生成する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json
import sys
from pathlib import Path
import numpy as np
from scipy.spatial import Delaunay
import mls_lpf_dem as lpf
from mls_quant_codec import encode_quant,decode_quant
from mls_dem_levels import write_json
from raw_dem_reference import RawDem,center_for
from sibson_surface import SibsonSurface
ROOT=Path('build/sibson-low-count-v1')

def worker(region):
 lpf.ROOT=ROOT;(ROOT/region).mkdir(parents=True,exist_ok=True)
 from mls_xy_placement import allocate
 for count in [4096,2048,1024,512,256,128]:
  src='six-xy';kind='natural-'+str(count)
  name='50-'+kind;folder=ROOT/region/name;folder.mkdir(exist_ok=True)
  if (folder/'info.json').exists():continue
  base=Path('build/mls-xy-placement-v1')/region/('50-'+src)
  original=json.loads((base/'info.json').read_text());p=decode_quant((base/'model.mlq1').read_bytes())
  filtered=lpf.FilteredDem(region,original['sigmaMeters']);raw=RawDem('build/terrain-cache',center_for(region))
  xy,placement=allocate(filtered,region,count)
  z=filtered.sample(xy);blob=encode_quant(np.column_stack((xy,z)),xy=.0625,zstep=.125)
  (folder/'model.mlq1').write_bytes(blob);points=decode_quant(blob)
  info=dict(region=region,targetError=50,n=1024,span=32000,controls=len(points),bytes=len(blob),totalBytes=len(blob),parentBytes=0,
   reconstruction='Sibson-natural-neighbor',reference='original-PNG-DEM',fittingReference='Gaussian-filtered-PNG-DEM',sigmaMeters=original['sigmaMeters'],
   sourceModel=str(base),placement=placement,xyCoordinatesUnchanged=False,zSource='sampled-LPF-target',zQuantizationMeters=.125,
   additionalDisplaySmoothing=False,filterHaloManifestSha256=filtered.digest)
  write_json(folder/'model-info.json',info)
  model=SibsonSurface(points)
  lpf.verify_export(region,50,name,raw,filtered,folder,model=model)
  dest=lpf.VIEW/region/name;tri=model.tri.astype('<u4')
  edges=np.unique(np.sort(np.vstack((tri[:,[0,1]],tri[:,[1,2]],tri[:,[2,0]])),axis=1),axis=0).astype('<u4')
  Path(str(dest)+'-controls.bin').write_bytes(points.astype('<f4').tobytes())
  Path(str(dest)+'-triangles.bin').write_bytes(tri.tobytes());Path(str(dest)+'-edges.bin').write_bytes(edges.tobytes())
  lpf.progress(region,name,'complete')

if __name__=='__main__':
 ROOT.mkdir(exist_ok=True)
 if len(sys.argv)>1:worker(sys.argv[1])
 else:
  import subprocess,concurrent.futures
  def run(r):
   with (ROOT/(r+'.log')).open('w',encoding='utf-8') as log:
    p=subprocess.Popen([sys.executable,'-X','utf8',__file__,r],stdout=log,stderr=subprocess.STDOUT)
    write_json(ROOT/(r+'.launch.json'),dict(pid=p.pid))
    if p.wait():raise RuntimeError(r+' の生成に失敗')
  write_json(ROOT/'progress.json',dict(stage='generating'))
  with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:list(pool.map(run,['fuji','yari','tsukuba']))
  write_json(ROOT/'progress.json',dict(stage='complete'))
