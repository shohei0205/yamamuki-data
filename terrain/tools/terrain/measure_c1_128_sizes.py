"""生成済み128kmタイルのサイズを同一スナップショットで集計する。"""
from pathlib import Path
from contextlib import closing
from concurrent.futures import ThreadPoolExecutor
import sqlite3,json,subprocess,hashlib,time,math
import numpy as np
from natural_c1_128 import ROOT,RUN,LEVELS
from test_fixed_lpf_regions import write
EXE=ROOT/'build/natural-fixed-lpf-target/release/terrain-natural-national.exe'
def measure(row):
 region,error,x,y,info=row;info=json.loads(info);folder=RUN/region/f'{error:g}'/f'{x}-{y}';result=dict(region=region,error=error,x=x,y=y,empty=bool(info.get('empty')),points=info.get('sourceControls',0),xyzBytes=info.get('sourceGeometryBytes',0),gpuBytes=(folder/'packet.bin').stat().st_size if (folder/'packet.bin').exists() else 0,gpuControls=info.get('controls',0),gpuPointBytes=info.get('pointBytes',0),gpuCellBytes=info.get('cellBytes',0),gpuNodeBytes=info.get('nodeBytes',0),gpuReferenceBytes=info.get('referenceBytes',0),riceBytes=0)
 if result['empty']:return result
 cached=folder/'xyz-rice-size.json'
 if cached.exists():result.update(json.loads(cached.read_text()));return result
 with closing(sqlite3.connect((folder/'work.sqlite3').as_uri()+'?mode=ro',uri=True)) as c:points=np.asarray(c.execute('SELECT p.qx,p.qy,l.qz FROM points p JOIN point_levels l ON l.id=p.id WHERE l.z=8 ORDER BY p.id').fetchall(),dtype=np.int64)
 source_origin=np.asarray(json.loads((RUN/region/'source/region.json').read_text())['origin']);origin=np.floor((source_origin+info['origin'])/.0625+.5).astype(np.int64);points[:,:2]-=origin
 inp=folder/'size-input.json';blob=folder/'control-points.nqc1';inp.write_text(json.dumps(dict(points=points.tolist())),encoding='utf-8');r=subprocess.run([str(EXE),'--pack',str(inp),'--output',str(blob)],cwd=ROOT,capture_output=True,text=True)
 if r.returncode:raise RuntimeError(r.stderr)
 result['riceBytes']=blob.stat().st_size;write(cached,dict(riceBytes=result['riceBytes'],origin_q=origin.tolist(),xyzStep=[.0625,.0625,.125],codec='NQC1 Rice/FOR/zero-run, independent',sha256=hashlib.sha256(blob.read_bytes()).hexdigest(),runtime_uses_this_blob=False));inp.unlink();return result
def run():
 started=time.perf_counter()
 with closing(sqlite3.connect((RUN/'queue.sqlite3').as_uri()+'?mode=ro',uri=True)) as c:
  c.execute('BEGIN');states=c.execute('SELECT state,count(*) FROM jobs GROUP BY state').fetchall();rows=c.execute("SELECT region,error,x,y,info FROM jobs WHERE state='ready' ORDER BY region,error DESC,y,x").fetchall();planned={(r,e):n for r,e,n in c.execute('SELECT region,error,count(*) FROM jobs GROUP BY region,error')}
 with ThreadPoolExecutor(max_workers=2) as pool:items=list(pool.map(measure,rows))
 summary=[]
 for region in ['fuji','yari','tsukuba']:
  for e,side in LEVELS:
   current=[v for v in items if v['region']==region and v['error']==e];a=dict(region=region,error=e,side=side,ready=len(current),planned=planned[(region,e)],empty=sum(v['empty'] for v in current))
   for key in ['points','xyzBytes','riceBytes','gpuBytes','gpuControls','gpuPointBytes','gpuCellBytes','gpuNodeBytes','gpuReferenceBytes']:a[key]=sum(v[key] for v in current)
   summary.append(a)
 report=dict(snapshot_states=states,ready=len(rows),planned=sum(planned.values()),seconds=time.perf_counter()-started,levels=summary,note='生成済みスナップショット。RiceはXYZのみ独立圧縮。GPU用情報・SQLite管理領域・元標高キャッシュ・差分圧縮は含まない。');write(RUN/'size-summary.json',report);print(json.dumps(report,ensure_ascii=False,indent=2))
if __name__=='__main__':run()
