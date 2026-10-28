"""誤差25・12.5 mの自然近傍モデルを既存と同じ基準で生成する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
from pathlib import Path
import sys,subprocess,concurrent.futures
import generate_sibson_error_driven as generator
from mls_dem_levels import write_json
ROOT=Path('build/sibson-error-fine-v1')

def worker(region):
 generator.ROOT=ROOT
 for error in [25,12.5]:generator.worker(region,error)

if __name__=='__main__':
 ROOT.mkdir(exist_ok=True)
 if len(sys.argv)>1:worker(sys.argv[1])
 else:
  def run(region):
   with (ROOT/(region+'.log')).open('w',encoding='utf-8') as log:
    p=subprocess.Popen([sys.executable,'-X','utf8',__file__,region],stdout=log,stderr=subprocess.STDOUT)
    write_json(ROOT/(region+'.launch.json'),dict(pid=p.pid))
    if p.wait():raise RuntimeError(region+' の生成に失敗')
  write_json(ROOT/'progress.json',dict(stage='generating',errors=[25,12.5]))
  with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:list(pool.map(run,['fuji','yari','tsukuba']))
  write_json(ROOT/'progress.json',dict(stage='complete',errors=[25,12.5]))
