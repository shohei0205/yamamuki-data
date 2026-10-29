"""3地域8km比較窓で全7品質の誤差適合データを生成。"""
import json,shutil,time,math,sqlite3
from concurrent.futures import ThreadPoolExecutor,as_completed
import generate_errorfit_c1_128 as fit
from natural_c1_128 import LEVELS
OUT=fit.ROOT/'build/natural-raw-c1-128-errorfit-levels-v1'
def tasks():
 for region in ['fuji','yari','tsukuba']:
  for error,side in LEVELS:
   for y in range(math.floor(60000/side),math.ceil(68000/side)):
    for x in range(math.floor(60000/side),math.ceil(68000/side)):yield region,error,x,y

def run():
 OUT.mkdir(exist_ok=True);lock=sqlite3.connect(OUT/'run-lock.sqlite3',timeout=0);lock.execute('BEGIN EXCLUSIVE');fit.OUT=OUT;jobs=list(tasks());done=[];errors={};started=time.monotonic()
 # 検証済み12.5m版はそのまま引き継ぎ、既存データは保持する。
 old=fit.ROOT/'build/natural-raw-c1-128-errorfit-v2'
 for region,error,x,y in jobs:
  if error!=12.5:continue
  src=old/region/'12.5'/f'{x}-{y}';dst=OUT/region/'12.5'/f'{x}-{y}';dst.mkdir(parents=True,exist_ok=True)
  if not (dst/'selection.json').exists():
   for name in ['packet.bin','packet.json','control-gradient.npz','selection.json']:shutil.copyfile(src/name,dst/name)
 def worker(job):
  region,error,x,y=job;fit.build(region,x,y,error);return job
 with ThreadPoolExecutor(max_workers=2) as pool:
  futures={pool.submit(worker,job):job for job in jobs}
  for future in as_completed(futures):
   job=futures[future]
   try:future.result();done.append(job)
   except Exception as e:errors[str(job)]=str(e);print('FAILED',job,str(e),flush=True)
   fit.write(OUT/'progress.json',dict(done=len(done),total=len(jobs),complete=len(done)==len(jobs),failed=errors,seconds=time.monotonic()-started))
if __name__=='__main__':run()
