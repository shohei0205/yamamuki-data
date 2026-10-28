"""粗い品質のXYを保持して、細かい品質へ点を追加する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
from pathlib import Path
import sys,subprocess,concurrent.futures
import generate_sibson_error_driven as generator
from mls_dem_levels import write_json
from mls_quant_codec import decode_quant
ROOT=Path('build/sibson-nested-v1')
ERRORS=[800,400,200,100,50,25,12.5]

def worker(region):
 generator.ROOT=ROOT;parent=None
 for error in ERRORS:
  generator.worker(region,error,seed_points=parent,nested=True)
  parent=decode_quant((ROOT/region/f'{error:g}-natural-nested{error:g}'/'model.mlq1').read_bytes())

if __name__=='__main__':
 ROOT.mkdir(exist_ok=True)
 if len(sys.argv)>1:worker(sys.argv[1])
 else:
  def run(region):
   with (ROOT/(region+'.log')).open('w',encoding='utf-8') as log:
    p=subprocess.Popen([sys.executable,'-X','utf8',__file__,region],stdout=log,stderr=subprocess.STDOUT)
    write_json(ROOT/(region+'.launch.json'),dict(pid=p.pid))
    if p.wait():raise RuntimeError(region+' の生成に失敗')
  write_json(ROOT/'progress.json',dict(stage='generating',errors=ERRORS))
  with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:list(pool.map(run,['fuji','yari','tsukuba']))
  write_json(ROOT/'progress.json',dict(stage='complete',errors=ERRORS))
