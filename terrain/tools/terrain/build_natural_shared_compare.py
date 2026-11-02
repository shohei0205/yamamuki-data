"""既存の全品質を全域へ統合し、分割・共通近傍傾きを比較生成する。"""
from pathlib import Path
import argparse,json,subprocess,time,sqlite3
import numpy as np
from benchmark_natural_global_scale import points_for
ROOT=Path(__file__).resolve().parents[2]
LEVELS=[(6,500,64000),(7,400,64000),(8,300,32000),(9,250,32000),(10,200,16000),(11,125,16000),(12,60,8000),(13,25,4000)]
def write(path,value):
 data=json.dumps(value,ensure_ascii=False,indent=2);temp=path.with_suffix('.next')
 temp.write_text(data,encoding='utf-8')
 for attempt in range(40):
  try:temp.replace(path);return
  except PermissionError:
   if attempt==39:raise
   time.sleep(.025)
def main():
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path,default=ROOT/'build/natural-shared-128-v1');p.add_argument('--source',type=Path,default=ROOT/'build/natural-xyz-hierarchy-z13-25-upquarter-local-v1');p.add_argument('--local-only',action='store_true');p.add_argument('--errors',type=int,nargs='*');a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
 lock=sqlite3.connect(a.output/'levels-run-lock.sqlite3',timeout=0);lock.execute('BEGIN EXCLUSIVE')
 exe=ROOT/'build/natural-fixed-lpf-target/release/terrain-natural-shared-compare.exe'
 levels=[dict(z=z,error=e,tileSide=side) for z,e,side in LEVELS]
 write(a.output/'levels.json',levels);reports=[];jobs=[(r,z,e,s) for z,e,s in LEVELS if not a.errors or e in a.errors for r in ['fuji','yari','tsukuba']]
 for region,z,error,side in jobs:
  dest=a.output/region if error==300 else a.output/region/'levels'/str(error);dest.mkdir(parents=True,exist_ok=True)
  report=dest/'report.json'
  if report.exists():
   previous=json.loads(report.read_text())
   if previous.get('complete') and previous.get('tileSide')==side:
    reports.append(dict(region=region,z=z,error=error,**previous));write(a.output/'summary-levels.json',reports);continue
  write(a.output/'progress.json',dict(stage='preparing',region=region,z=z,error=error,completed=len(reports),total=len(jobs)))
  points=points_for(a.source,region,z)
  points=np.concatenate([points,[[-2000,-2000,0],[130000,-2000,0],[-2000,130000,0],[130000,130000,0]]]);path=dest/'input.xyz64';points.astype('<f8').tofile(path);del points
  started=time.time();logpath=dest/'run.log'
  with logpath.open('w',encoding='utf-8') as log:
   process=subprocess.Popen([str(exe),'--input',str(path),'--output',str(dest),'--tile-side',str(side)]+(['--local-only'] if a.local_only else []),stdout=log,stderr=subprocess.STDOUT)
   while process.poll() is None:
    lines=logpath.read_text(encoding='utf-8',errors='replace').splitlines()
    write(a.output/'progress.json',dict(stage='generating',region=region,z=z,error=error,pid=process.pid,completed=len(reports),total=len(jobs),tileSide=side,tilesDone=sum(l.startswith('完了 ') for l in lines),tilesTotal=(128000//side)**2,detail=lines[-1] if lines else '全域接続を準備中',elapsed=time.time()-started))
    time.sleep(1)
  if process.returncode:
   write(a.output/'progress.json',dict(stage='failed',region=region,z=z,error=error,exitCode=process.returncode,log=str(logpath)));raise RuntimeError(f'{region}/{error}: {process.returncode}, {logpath}')
  reports.append(dict(region=region,z=z,error=error,**json.loads(report.read_text())));write(a.output/'summary-levels.json',reports)
 write(a.output/'progress.json',dict(stage='complete',completed=len(reports),total=len(jobs),reports=reports))
 lock.close()
if __name__=='__main__':main()
