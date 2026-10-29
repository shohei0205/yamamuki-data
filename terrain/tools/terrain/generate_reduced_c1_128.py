"""800m・128kmの制御点数を1/2、1/4、1/8にしたLPFなしC1比較。"""
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor
import hashlib,shutil,time,json
import natural_c1_128 as worker
from test_fixed_lpf_regions import ROOT,write
BASE=ROOT/'build/natural-raw-c1-128-v1'
def run():
 started=time.monotonic();reports=[];release=ROOT/'build/natural-fixed-lpf-target/release/terrain-natural-national.exe';tag=hashlib.sha256(release.read_bytes()).hexdigest()[:16]
 for name,factor in [('half',.5),('quarter',.25),('eighth',.125)]:
  worker.RUN=ROOT/f'build/natural-raw-c1-128-{name}-v1';worker.RUN.mkdir(exist_ok=True);worker.SOURCE_RUN=BASE;worker.BUDGET_FACTOR=factor;worker.ENGINE=worker.RUN/f'engine-{tag}.exe'
  if not worker.ENGINE.exists():shutil.copyfile(release,worker.ENGINE)
  def build(region):
   folder=worker.paths(region,800,0,0)
   if (folder/'packet.json').exists():return json.loads((folder/'packet.json').read_text())
   report=worker.build(region,800,0,0);base=json.loads((BASE/region/'800/0-0/packet.json').read_text());expected=max(4,round((base['sourceControls']-4)*factor))
   if report['sourceControls']-4!=expected:raise RuntimeError('制御点数が目標と一致しません')
   report.update(density=name,baselineControls=base['sourceControls']-4,controlBudget=expected);write(folder/'packet.json',report);return report
  with ThreadPoolExecutor(max_workers=3) as pool:rows=list(pool.map(build,['fuji','yari','tsukuba']))
  reports.extend(rows);write(BASE/'reduced-800-progress.json',dict(completed=[r['density'] for r in reports],reports=reports,seconds=time.monotonic()-started))
 write(BASE/'reduced-800-complete.json',dict(complete=True,reports=reports,seconds=time.monotonic()-started))
if __name__=='__main__':run()
