"""点数を地域ごとに固定し、LPFのσだけを400/200/100/50 mで比較する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
from pathlib import Path
from contextlib import closing
from concurrent.futures import ThreadPoolExecutor
import json,sqlite3,subprocess,shutil,time
from test_fixed_lpf_regions import ROOT,preview,write
BASE=ROOT/'build/natural-fixed-lpf-regions-v1';SCALE=ROOT/'build/natural-scale-placement-regions-v1';RUN=ROOT/'build/natural-same-budget-lpf-v1'
def run():
 RUN.mkdir(exist_ok=True);engine=RUN/'engine.exe'
 if not engine.exists():shutil.copyfile(ROOT/'build/natural-fixed-lpf-target/release/terrain-natural-national.exe',engine)
 start=time.monotonic()
 def worker(name):
  original=BASE/name;region=json.loads((original/'region.json').read_text());budget=json.loads((SCALE/name/'preview-800.json').read_text())['points'];reports=[]
  for sigma,source_z in [(400,8),(200,9),(100,10),(50,11)]:
   folder=ROOT/f'build/natural-budget-sigma{sigma}-v1'/name;folder.mkdir(parents=True,exist_ok=True)
   if (folder/'preview-800.json').exists():reports.append(json.loads((folder/'preview-800.json').read_text()));continue
   settings=json.loads((BASE/'settings.json').read_text());settings.update(placement_policy='lpf-scale-poisson',fixed_point_budget=budget,placement_only_z=8,comparison_sigma=sigma);write(folder/'settings-input.json',settings);write(folder/'region.json',region)
   def native(mode,*extra):
    with (folder/f'{mode}.stdout.log').open('ab') as a,(folder/f'{mode}.stderr.log').open('ab') as b:
     p=subprocess.Popen([str(engine),'--'+mode,'--output',str(folder),'--workers','1',*map(str,extra)],cwd=ROOT,stdout=a,stderr=b,creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0));write(folder/'launch.json',dict(pid=p.pid,mode=mode,engine=str(engine)))
    if p.wait():raise RuntimeError(f'{name} sigma{sigma}: {mode} failed')
   native('inventory','--source',ROOT/'build/terrain-lod.sqlite3','--cache',ROOT/'build/terrain-cache','--settings',folder/'settings-input.json','--window',*region['window'],*(['--resume'] if (folder/'work.sqlite3').exists() else []))
   with closing(sqlite3.connect(folder/'work.sqlite3')) as dest,closing(sqlite3.connect((original/'work.sqlite3').as_uri()+'?mode=ro',uri=True)) as src:
    if dest.execute('SELECT x,y,sha FROM inputs ORDER BY y,x').fetchall()!=src.execute('SELECT x,y,sha FROM inputs ORDER BY y,x').fetchall():raise ValueError('LPF再利用の元PNGが不一致')
    with dest:
     for _,z,x,y,path,sha in src.execute("SELECT * FROM artifacts WHERE stage='lpf' AND z=?",(source_z,)):dest.execute("INSERT OR REPLACE INTO artifacts VALUES('lpf',8,?,?,?,?)",(x,y,path,sha))
     dest.execute('INSERT OR REPLACE INTO metadata VALUES(?,?)',('lpf-sigma-z8',str(sigma)))
   t=time.monotonic();native('place-scale');seconds=time.monotonic()-t
   with closing(sqlite3.connect(folder/'work.sqlite3')) as c:
    count=c.execute('SELECT count(*) FROM point_levels WHERE z=8 AND flags<>1').fetchone()[0]
    if count!=budget:raise ValueError(f'固定点数が不一致: {count} != {budget}')
   preview(folder,name,8,800,region['center'],engine);path=folder/'preview-800.json';r=json.loads(path.read_text());r.update(sigma=sigma,fit_error_m=None,max_error_guaranteed=False,fixed_budget=budget,placement_seconds=seconds);write(path,r);reports.append(r);write(RUN/f'{name}-progress.json',dict(completed_sigmas=[v['sigma'] for v in reports],budget=budget))
  return reports
 with ThreadPoolExecutor(max_workers=3) as pool:reports=list(pool.map(worker,['fuji','yari','tsukuba']))
 write(RUN/'complete.json',dict(complete=True,seconds=time.monotonic()-start,reports=reports))
if __name__=='__main__':run()
