"""保存済みの制御点・勾配を変えず、RustでGPU索引だけを別フォルダーへ再構築する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import argparse,json,sqlite3,struct,subprocess,time,hashlib
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor,as_completed
import numpy as np
ROOT=Path(__file__).resolve().parents[2]
def write(path,data):
 b=json.dumps(data,ensure_ascii=False,indent=2).encode();tmp=path.with_suffix('.next');tmp.write_bytes(b)
 for i in range(40):
  try:tmp.replace(path);return
  except PermissionError:
   if i==39:raise
   time.sleep(.025)
def repack(source,output,exe,key):
 z,x,y=key;src=source/f'tiles/{z}/{x}/{y}';dst=output/f'tiles/{z}/{x}/{y}';dst.mkdir(parents=True,exist_ok=True);meta=json.loads((src/'packet.json').read_text());signature=hashlib.sha256((src/'packet.json').read_bytes()).hexdigest();report=dst/'repack.json'
 if report.exists():
  old=json.loads(report.read_text())
  if old.get('sourceSignature')==signature:return old
 start=time.perf_counter();oldsize=(src/'packet.bin').stat().st_size if (src/'packet.bin').exists() else 0
 if meta.get('empty'):
  packet=(src/'packet.bin').read_bytes() if (src/'packet.bin').exists() else b'';newmeta=meta
 else:
  if (src/'control-gradient.json').exists():
   data=json.loads((src/'control-gradient.json').read_text());points=data['points'];gradient=data['gradient']
  else:
   gradient_path=src/'control-gradient.npz'
   if not gradient_path.exists():
    reuse=json.loads((source/'hierarchy.json').read_text())['reuseSource']
    gradient_path=Path(reuse)/f'tiles/{z}/{x}/{y}/control-gradient.npz'
   with np.load(gradient_path) as data:points=data['points'].tolist();gradient=data['gradient'].tolist()
  assert len(points)==len(gradient)
  payload=dict(operation='packet',points=points,gradient=gradient,bounds=meta['bounds'],metadata=meta);inputpath=dst/'repack-input.json';write(inputpath,payload)
  process=subprocess.run([str(exe),'--evaluate',str(inputpath),'--output',str(dst/'packet.bin')],capture_output=True,text=True,encoding='utf-8')
  if process.returncode:raise RuntimeError(f'{key}: {process.stderr[-1500:]}')
  packet=(dst/'packet.bin').read_bytes();n=struct.unpack_from('<I',packet)[0];newmeta=json.loads(packet[4:4+n]);inputpath.unlink()
 write(dst/'packet.json',newmeta)
 if meta.get('empty'):(dst/'packet.bin').write_bytes(packet)
 result=dict(z=z,x=x,y=y,sourceSignature=signature,oldBytes=oldsize,newBytes=len(packet),seconds=time.perf_counter()-start,nodes=newmeta.get('nodeCount',0),candidateLists=newmeta.get('uniqueCandidateLists',0));write(report,result);return result
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--source',type=Path,default=ROOT/'build/natural-xyz-hierarchy-v1');p.add_argument('--output',type=Path,default=ROOT/'build/natural-xyz-hierarchy-gpu-v2');p.add_argument('--exe',type=Path,default=ROOT/'build/natural-fixed-lpf-target/release/terrain-natural-regional.exe');p.add_argument('--workers',type=int,default=max(1,(os.cpu_count() or 2)//2));p.add_argument('--min-z',type=int,default=6);p.add_argument('--max-z',type=int,default=14);a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
 lock=sqlite3.connect(a.output/'run-lock.sqlite3',timeout=1);lock.execute('BEGIN EXCLUSIVE')
 with sqlite3.connect(a.source.resolve().joinpath('queue.sqlite3').as_uri()+'?mode=ro',uri=True) as c:keys=c.execute("select z,x,y from jobs where state='ready' and z between ? and ? order by z,y,x",(a.min_z,a.max_z)).fetchall()
 progress=dict(total=len(keys),ready=0,failed=[],oldBytes=0,newBytes=0,source=str(a.source),started=time.time(),complete=False);write(a.output/'progress.json',progress)
 with ThreadPoolExecutor(max_workers=a.workers) as pool:
  tasks={pool.submit(repack,a.source,a.output,a.exe.resolve(),key):key for key in keys}
  for future in as_completed(tasks):
   try:
    r=future.result();progress['ready']+=1;progress['oldBytes']+=r['oldBytes'];progress['newBytes']+=r['newBytes'];progress['latest']=r
   except Exception as e:progress['failed'].append(dict(key=tasks[future],message=str(e)))
   progress['updated']=time.time();write(a.output/'progress.json',progress)
 progress['complete']=not progress['failed'];write(a.output/'progress.json',progress);print(json.dumps(progress,ensure_ascii=False));raise SystemExit(bool(progress['failed']))
