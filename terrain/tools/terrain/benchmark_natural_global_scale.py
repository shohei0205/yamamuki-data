"""既存の完成制御点で全域構築と分割の資源量を測る。新たなDEM適合ではない。"""
import argparse,json,math,sqlite3,subprocess,time,ctypes
from pathlib import Path
import numpy as np
from sibson_native_codec import decode_independent
ROOT=Path(__file__).resolve().parents[2]
WORLD=40075016.68557849

def points_for(source,region,z):
 settings=json.loads((source/'settings.json').read_text(encoding='utf-8-sig'))
 bounds=np.array(next(r['bounds'] for r in settings['regions'] if r['region']==region))
 result=[]
 with sqlite3.connect(source/'queue.sqlite3') as c:
  rows=c.execute("select x,y from region_tiles where region=? and z=?",(region,z)).fetchall()
 for x,y in rows:
  f=source/f'tiles/{z}/{x}/{y}/control-xyz.nqc1'
  if not f.exists():continue
  a=decode_independent(f.read_bytes()).astype(np.float64)
  side=WORLD/(1<<z);origin=np.floor(np.array([x*side,y*side])*16+0.5)/16
  a[:,:2]=a[:,:2]/16+origin;a[:,2]/=8
  mask=(a[:,0]>=max(x*side,bounds[0]))&(a[:,0]<min((x+1)*side,bounds[2]))&(a[:,1]>=max(y*side,bounds[1]))&(a[:,1]<min((y+1)*side,bounds[3]))
  result.append(a[mask])
 a=np.concatenate(result);a[:,:2]-=bounds[:2]
 _,index=np.unique(a[:,:2],axis=0,return_index=True)
 return a[index]

def prepare(a,span,repeat=False):
 if repeat:
  k=round(span/128000);a=np.concatenate([a+np.array([x*128000,y*128000,0]) for y in range(k) for x in range(k)])
 else:
  lo=(128000-span)/2;a=a[(a[:,0]>=lo)&(a[:,0]<lo+span)&(a[:,1]>=lo)&(a[:,1]<lo+span)].copy();a[:,:2]-=lo
 # 外周の補間を閉じるための番兵。測定用であり地形の品質判定には使わない。
 return np.concatenate([a,np.array([[-100,-100,0],[span+100,-100,0],[-100,span+100,0],[span+100,span+100,0]])])

def main():
 p=argparse.ArgumentParser();p.add_argument('--source',type=Path,default=ROOT/'build/natural-xyz-hierarchy-z13-25-upquarter-local-v1');p.add_argument('--output',type=Path,default=ROOT/'build/natural-global-scale-v1');p.add_argument('--auxiliary',action='store_true');p.add_argument('--dense-stress',action='store_true');args=p.parse_args();args.output.mkdir(exist_ok=True,parents=True)
 exe=ROOT/'build/natural-fixed-lpf-target/release/terrain-natural-scale-bench.exe'
 cases=[]
 for region in ['fuji','yari','tsukuba']:
  print('読込',region,flush=True);a=points_for(args.source,region,13)
  for span in [32000,64000,128000]:cases.append((f'{region}-z13-{span//1000}km',prepare(a,span),span,False))
 # 拡大試験は同じ地形密度を複製。全国実データを生成したとの意味ではない。
 a=points_for(args.source,'yari',11)
 for span in [256000,512000]:cases.append((f'synthetic-yari-z11-{span//1000}km',prepare(a,span,True),span,True))
 if args.dense_stress:cases.append(('synthetic-yari-z13-256km',prepare(points_for(args.source,'yari',13),256000,True),256000,True))
 summary=[]
 for name,a,span,synthetic in cases:
  path=args.output/f'{name}.xyz64';a.astype('<f8').tofile(path)
  if args.auxiliary:name+='-aux'
  out=args.output/f'{name}.json';log=args.output/f'{name}.log'
  if len(a)>4000000:summary.append(dict(name=name,skipped='400万点上限',points=len(a)));continue
  print('開始',name,len(a),flush=True)
  with log.open('w',encoding='utf-8') as stream:
   proc=subprocess.Popen([str(exe),'--input',str(path),'--output',str(out),'--side',str(span)]+(['--auxiliary'] if args.auxiliary else []),stdout=stream,stderr=subprocess.STDOUT)
   launch=dict(name=name,pid=proc.pid,points=len(a),synthetic=synthetic,started=time.time());(args.output/('aux-progress.json' if args.auxiliary else 'progress.json')).write_text(json.dumps(dict(stage='running',current=launch,completed=summary),ensure_ascii=False,indent=2),encoding='utf-8')
   try:code=proc.wait(timeout=600)
   except subprocess.TimeoutExpired:proc.kill();proc.wait();code=-1
  record=dict(name=name,exit_code=code,synthetic=synthetic,input_bytes=path.stat().st_size)
  if out.exists():record.update(json.loads(out.read_text()))
  summary.append(record);(args.output/('aux-summary.json' if args.auxiliary else 'summary.json')).write_text(json.dumps(summary,ensure_ascii=False,indent=2),encoding='utf-8');print('終了',name,code,flush=True)
 (args.output/('aux-progress.json' if args.auxiliary else 'progress.json')).write_text(json.dumps(dict(stage='complete',completed=summary),ensure_ascii=False,indent=2),encoding='utf-8')
if __name__=='__main__':main()
