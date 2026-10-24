"""接合で実際に必要となった候補だけを、共通の余白メッシュへ事前挿入する。"""
import argparse,hashlib,json,sqlite3,subprocess
import overlap_compare as o
from test_overlap_compare import check
EXE=o.ROOT/'build/terrain-q-cache-target/release/terrain-overlap-compare.exe'
CONFIGS=[('dual',[50,100]),('dual',[12.5,800]),('quad',[25,50,100,200]),('quad',[800,12.5,800,12.5]),('tee',[200,50,100]),('tee',[800,12.5,25]),('partial',[200,50,400,100]),('partial',[800,12.5,25,400])]
def execute(request):
 r=subprocess.run([str(EXE)],input=json.dumps(request).encode(),capture_output=True,creationflags=subprocess.CREATE_NO_WINDOW)
 if r.returncode:raise RuntimeError(r.stderr.decode(errors='replace'))
 return json.loads(r.stdout)
def pk(p):return tuple(round(v,6) for v in p)
def tile(mesh,region,error,rect,width,support):
 x,y,u,v=rect;lo=[max(0,x-width),max(0,y-width)];hi=[min(16000,u+width),min(16000,v+width)]
 fs=[f for f in mesh['faces'] if all(max(mesh['points'][i][k] for i in f)>=lo[k] and min(mesh['points'][i][k] for i in f)<=hi[k] for k in (0,1))]
 ids=sorted({i for f in fs for i in f});remap={v:i for i,v in enumerate(ids)}
 return dict(meshKey=mesh['meshKey'],bounds=rect,points=[mesh['points'][i] for i in ids],faces=[[remap[i] for i in f] for f in fs],support=support)
def build(width):
 report=[]
 for region in ['fuji','yari','tsukuba']:
  with sqlite3.connect(o.SOURCE.as_uri()+'?mode=ro',uri=True) as db:
   sources={e:json.loads(raw) for e,raw in db.execute("select error,preview from meshes where region=? and method='decimated-boundary'",(region,))}
  meshes={e:dict(m,meshKey=region+':'+str(e)+':original') for e,m in sources.items()};chosen={e:{} for e in sources}
  for iteration in range(4):
   count=0
   for layout,errors in CONFIGS:
    if max(errors)==800:continue
    ts=[]
    for e,b in zip(errors,o.bounds(layout)):
     old,_=o.prepare_tile(region,e,b,width);current=tile(meshes[e],region,e,b,width,old['support']);ts.append(current)
    m=execute(dict(tiles=ts,width=width,local=True,partial=False));check(m,ts)
    if not m['stats']['qualitySatisfied']:continue
    originals={pk(p) for t in ts for p in t['points']}
    used={pk(p) for p in m['points']}-originals
    for e,t in zip(errors,ts):
     for p in t['support']:
      k=pk(p)
      if k in used and k not in chosen[e]:chosen[e][k]=p;count+=1
   print(region,width,'反復',iteration+1,'補修点',count,flush=True)
   if not count:break
   for e,source in sources.items():
    mesh=execute(dict(bake=True,pool=sources[12.5]['points'],tile=dict(bounds=[0,0,16000,16000],points=source['points'],faces=source['faces'],support=list(chosen[e].values()))))
    # 全地域を共通に補修してから分け、同レベルの余白形状を一致させる。
    raw=json.dumps(mesh,separators=(',',':')).encode();mesh['meshKey']='halo-baked-v1:'+hashlib.sha256(raw).hexdigest();meshes[e]=mesh
  with sqlite3.connect(o.OUT/'tiles.sqlite3') as db:
   db.execute('create table if not exists refined_meshes_v1(region text,error real,width real,body blob,primary key(region,error,width))')
   for e,m in meshes.items():db.execute('insert or replace into refined_meshes_v1 values(?,?,?,?)',(region,e,width,json.dumps(m,separators=(',',':')).encode()))
  for layout,errors in CONFIGS:
   ts=[tile(meshes[e],region,e,b,width,[]) for e,b in zip(errors,o.bounds(layout))]
   m=execute(dict(tiles=ts,width=width,local=True,partial=False));check(m,ts)
   report.append(dict(region=region,width=width,layout=layout,errors=errors,**m['stats'],inputBytes=sum(len(json.dumps(t,separators=(',',':')).encode()) for t in ts)))
  print(region,width,'保存済み・角度合格',sum(r['qualitySatisfied'] for r in report if r['region']==region),'/',len(CONFIGS),flush=True)
 (o.OUT/f'halo-baked-{width}-audit.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--width',type=int,default=2000);build(p.parse_args().width)
