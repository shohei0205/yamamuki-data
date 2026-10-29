"""20～1280mの完成済みタイルだけをスナップショット集計。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json,sqlite3,time
from concurrent.futures import ProcessPoolExecutor
import measure_errorfit_full_sizes as m
from errorfit_20_series import OUT,LEVELS

def measure(row):
 m.OUT=OUT
 return m.measure(row)
def run():
 with sqlite3.connect(OUT.as_posix()+'/queue.sqlite3') as c:
  rows=c.execute("SELECT region,error,x,y,info FROM jobs WHERE state='ready'").fetchall()
  states=dict(c.execute('SELECT state,count(*) FROM jobs GROUP BY state'))
 start=time.time();levels=[]
 with ProcessPoolExecutor(max_workers=max(1,(os.cpu_count() or 2)//2)) as pool:results=list(pool.map(measure,rows,chunksize=32))
 for e,side in LEVELS:
  a=[r for r in results if r['error']==e];r=dict(error=e,sideKm=side/1000,ready=len(a),planned=3*round(128000/side)**2)
  for key in ['points','riceBytes','gpuBytes','empty']:r[key]=sum(v[key] for v in a)
  levels.append(r)
 report=dict(snapshotTime=start,states=states,levels=levels,complete=len(rows)==16383,seconds=time.time()-start)
 m.write(OUT/'size-report-rice-snapshot.json',report)
 print(json.dumps(report,indent=2),flush=True)
if __name__=='__main__':run()
