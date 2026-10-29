"""LPFなし。十分な自由配置を先に作り、元DEM誤差800 mを全有効画素で検証する。"""
from contextlib import closing
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import json,sqlite3,subprocess,shutil,time
from test_fixed_lpf_regions import ROOT,write,preview
BASE=ROOT/'build/natural-fixed-lpf-regions-v1';RUN=ROOT/'build/natural-raw-shape800-v1'
def run():
 RUN.mkdir(exist_ok=True);engine=RUN/'engine.exe'
 if not engine.exists():shutil.copyfile(ROOT/'build/natural-fixed-lpf-target/release/terrain-natural-national.exe',engine)
 started=time.monotonic()
 def worker(name):
  out=RUN/name;out.mkdir(exist_ok=True);region=json.loads((BASE/name/'region.json').read_text());write(out/'region.json',region)
  cfg=json.loads((BASE/'settings.json').read_text());cfg.update(lpf_policy='none',fit_reference='lpf',fit_error_ratio=1.,placement_policy='lpf-scale-poisson',placement_only_z=8);write(out/'settings-input.json',cfg)
  def native(mode,*extra):
   with (out/f'{mode}.stdout.log').open('ab') as a,(out/f'{mode}.stderr.log').open('ab') as b:
    p=subprocess.Popen([str(engine),'--'+mode,'--output',str(out),'--workers','2',*map(str,extra)],cwd=ROOT,stdout=a,stderr=b,creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0));write(out/'launch.json',dict(pid=p.pid,mode=mode,engine=str(engine)))
   if p.wait():raise RuntimeError(f'{name}/{mode} failed: {out}')
  native('inventory','--source',ROOT/'build/terrain-lod.sqlite3','--cache',ROOT/'build/terrain-cache','--settings',out/'settings-input.json','--window',*region['window'],*(['--resume'] if (out/'work.sqlite3').exists() else []));native('lpf')
  with closing(sqlite3.connect(out/'work.sqlite3')) as c:already=c.execute("SELECT value FROM metadata WHERE key='raw-shape-complete'").fetchone() or c.execute("SELECT 1 FROM tasks WHERE stage='fit' AND z=8 GROUP BY z HAVING count(*)>0 AND sum(state<>'passed')=0").fetchone()
  if not already:
   native('place-scale')
   with closing(sqlite3.connect(out/'work.sqlite3')) as c,c:
    initial=c.execute('SELECT count(*) FROM point_levels WHERE z=8 AND flags<>1').fetchone()[0];c.execute("INSERT OR REPLACE INTO metadata VALUES('fit-phase-z8','fitting')");c.execute("INSERT OR REPLACE INTO metadata VALUES('fit-seeded-z8','true')");c.execute("INSERT OR REPLACE INTO metadata VALUES('raw-initial-points',?)",(str(initial),))
   native('fit')
  with closing(sqlite3.connect(out/'work.sqlite3')) as c:
   rows=c.execute("SELECT state,info FROM tasks WHERE stage='fit' AND z=8").fetchall();info=[json.loads(r[1]) for r in rows]
   total=c.execute('SELECT sum(valid) FROM inputs').fetchone()[0]
   if not rows or any(r[0]!='passed' for r in rows) or sum(v['valid'] for v in info)!=total or any(v['maximum_error_bound']>800 or v['reused_cells']!=0 for v in info):raise RuntimeError('元DEM全有効画素の最終監査が未達')
   report=dict(complete=True,quality_error_m=800,sigma=0,max_error_guaranteed_on_source_pixels=True,valid_source_pixels=total,maximum_error_bound=max(v['maximum_error_bound'] for v in info),initial_points=int(c.execute("SELECT value FROM metadata WHERE key='raw-initial-points'").fetchone()[0]),points=c.execute('SELECT count(*) FROM point_levels WHERE z=8 AND flags<>1').fetchone()[0])
   with c:c.execute("INSERT OR REPLACE INTO metadata VALUES('raw-shape-complete','true')")
  preview(out,name,8,800,region['center'],engine);path=out/'preview-800.json';p=json.loads(path.read_text());p.update(sigma=0,fit_error_m=800,max_error_guaranteed=True,source_pixel_audit=report,note='LPFなし。量子化後の曲面と元DEMを全有効画素で照合。表示上の誤差値は別の513x513格子での参考値。');write(path,p);write(out/'test-complete.json',report);return report
 with ThreadPoolExecutor(max_workers=3) as pool:reports=list(pool.map(worker,['fuji','yari','tsukuba']))
 write(RUN/'complete.json',dict(complete=True,seconds=time.monotonic()-started,reports=reports))
if __name__=='__main__':run()
