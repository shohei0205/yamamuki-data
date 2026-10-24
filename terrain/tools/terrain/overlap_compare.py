"""余白タイルの事前保存とRust製マージ試験の配信。"""
import argparse,json,sqlite3,subprocess,time,math,threading,hashlib
from pathlib import Path
from http.server import SimpleHTTPRequestHandler,ThreadingHTTPServer
from urllib.parse import urlsplit,parse_qs
ROOT=Path(__file__).resolve().parents[2]
SOURCE=ROOT/'build/decimation-compare-20261023/comparison.sqlite3'
OUT=ROOT/'build/overlap-compare-20261023'
LOCK=threading.Lock()
def bounds(layout,axis=0):
 if layout=='quad':return [[0,0,8000,8000],[8000,0,16000,8000],[0,8000,8000,16000],[8000,8000,16000,16000]]
 if layout=='tee':return [[0,0,8000,16000],[8000,0,16000,8000],[8000,8000,16000,16000]]
 if layout=='partial':return [[0,0,8000,16000],[8000,4000,16000,12000],[8000,0,16000,4000],[8000,12000,16000,16000]]
 assert layout=='dual'
 return [[0,0,8000,16000],[8000,0,16000,16000]] if axis==0 else [[0,0,16000,8000],[0,8000,16000,16000]]
def distance(p,a,b):
 dx,dy=b[0]-a[0],b[1]-a[1];t=max(0,min(1,((p[0]-a[0])*dx+(p[1]-a[1])*dy)/(dx*dx+dy*dy)))
 return math.hypot(p[0]-a[0]-t*dx,p[1]-a[1]-t*dy)
def prepare_tile(region,error,rect,width):
 OUT.mkdir(exist_ok=True);identity=json.dumps(rect);dbpath=OUT/'tiles.sqlite3'
 with LOCK,sqlite3.connect(dbpath) as db:
  db.execute('create table if not exists tiles_v3(region text,error real,bounds text,width real,body blob,primary key(region,error,bounds,width))')
  row=db.execute('select body from tiles_v3 where region=? and error=? and bounds=? and width=?',(region,error,identity,width)).fetchone()
 if row:return json.loads(row[0]),len(row[0])
 with sqlite3.connect(SOURCE.as_uri()+'?mode=ro',uri=True) as db:
  raw_mesh=db.execute("select preview from meshes where region=? and method='decimated-boundary' and error=?",(region,error)).fetchone()[0];mesh_key=region+":"+format(float(error),".12g")+":"+hashlib.sha256(raw_mesh.encode()).hexdigest();mesh=json.loads(raw_mesh);reference=json.loads(db.execute("select preview from meshes where region=? and method='decimated-boundary' and error=12.5",(region,)).fetchone()[0])
 x,y,u,v=rect;expanded=[max(0,x-width),max(0,y-width),min(16000,u+width),min(16000,v+width)]
 def intersects(f):
  ps=[mesh['points'][i] for i in f]
  return max(p[0] for p in ps)>=expanded[0] and min(p[0] for p in ps)<=expanded[2] and max(p[1] for p in ps)>=expanded[1] and min(p[1] for p in ps)<=expanded[3]
 fs=[f for f in mesh['faces'] if intersects(f)];ids=sorted(set(i for f in fs for i in f));remap={v:i for i,v in enumerate(ids)}
 edges=[(a,b) for a,b in [([x,y],[u,y]),([u,y],[u,v]),([u,v],[x,v]),([x,v],[x,y])] if not(a[0]==b[0] and a[0] in (0,16000) or a[1]==b[1] and a[1] in (0,16000))]
 existing={(mesh['points'][i][0],mesh['points'][i][1]) for i in ids}
 support=[p for p in reference['points'] if (p[0],p[1]) not in existing and expanded[0]<=p[0]<=expanded[2] and expanded[1]<=p[1]<=expanded[3] and any(distance(p,a,b)<=width for a,b in edges)]
 body=dict(meshKey=mesh_key,bounds=rect,points=[mesh['points'][i] for i in ids],faces=[[remap[i] for i in f] for f in fs],support=support)
 raw=json.dumps(body,separators=(',',':')).encode()
 with LOCK,sqlite3.connect(dbpath) as db:db.execute('insert or replace into tiles_v3 values(?,?,?,?,?)',(region,error,identity,width,raw))
 return body,len(raw)
def prepare_refined_tile(region,error,rect,width):
 if width!=2000:raise ValueError('事前補修メッシュは余白2000 mで比較してください')
 with LOCK,sqlite3.connect(OUT/'tiles.sqlite3') as db:
  db.execute('create table if not exists tiles_refined_v1(region text,error real,bounds text,width real,body blob,primary key(region,error,bounds,width))')
  row=db.execute('select body from tiles_refined_v1 where region=? and error=? and bounds=? and width=?',(region,error,json.dumps(rect),width)).fetchone()
  if row:
   current=db.execute("select json_extract(cast(body as text),'$.meshKey') from refined_meshes_v1 where region=? and error=? and width=?",(region,error,width)).fetchone()
   cached=json.loads(row[0])
   if current and cached['meshKey']==current[0]:return cached,len(row[0])
  row=db.execute('select body from refined_meshes_v1 where region=? and error=? and width=?',(region,error,width)).fetchone()
 if not row:raise ValueError('事前補修メッシュがまだ生成されていません')
 from bake_overlap_halo import tile
 body=tile(json.loads(row[0]),region,error,rect,width,[]);raw=json.dumps(body,separators=(',',':')).encode()
 with LOCK,sqlite3.connect(OUT/'tiles.sqlite3') as db:db.execute('insert or replace into tiles_refined_v1 values(?,?,?,?,?)',(region,error,json.dumps(rect),width,raw))
 return body,len(raw)

def prepare_layer_tile(region,error,rect,layers):
 from overlap_layers import extract
 assert layers in (1,2,3,4)
 with LOCK,sqlite3.connect(OUT/'tiles.sqlite3') as db:
  db.execute('create table if not exists tiles_layers_v1(region text,error real,bounds text,layers integer,mesh_key text,body blob,primary key(region,error,bounds,layers))')
  current=db.execute("select json_extract(cast(body as text),'$.meshKey') from refined_meshes_v1 where region=? and error=? and width=2000",(region,error)).fetchone()
  if not current:raise ValueError('共通の事前補修メッシュがありません')
  row=db.execute('select mesh_key,body from tiles_layers_v1 where region=? and error=? and bounds=? and layers=?',(region,error,json.dumps(rect),layers)).fetchone()
  if row and row[0]==current[0]:return json.loads(row[1]),len(row[1])
  raw=db.execute('select body from refined_meshes_v1 where region=? and error=? and width=2000',(region,error)).fetchone()[0]
 body=extract(json.loads(raw),rect,layers);raw=json.dumps(body,separators=(',',':')).encode()
 with LOCK,sqlite3.connect(OUT/'tiles.sqlite3') as db:db.execute('insert or replace into tiles_layers_v1 values(?,?,?,?,?,?)',(region,error,json.dumps(rect),layers,body['meshKey'],raw))
 return body,len(raw)

def mark_changed_faces(mesh,tiles):
 # 頂点番号や面の向きではなく、元の座標3点の集合で比較する。
 def face_key(points,f):return tuple(sorted(tuple(round(v,6) for v in points[i]) for i in f))
 original={face_key(t['points'],f) for t in tiles for f in t['faces']}
 mesh['changedFaces']=[face_key(mesh['points'],f) not in original for f in mesh['faces']]
 changed=sum(mesh['changedFaces']);mesh['stats'].update(changedTriangles=changed,unchangedTriangles=len(mesh['faces'])-changed)

def make(region,errors,layout='dual',axis=0,width=1000,local=True,candidates=True,refined=False,layers=0):
 assert region in ['fuji','yari','tsukuba'] and width in [500,1000,2000,4000] and axis in (0,1)
 rects=bounds(layout,axis);assert len(errors)==len(rects) and all(e is None or e in [12.5,25,50,100,200,400,800] for e in errors)
 slots=[i for i,e in enumerate(errors) if e is not None]
 prepared=[prepare_layer_tile(region,errors[i],rects[i],layers) if layers else (prepare_refined_tile if refined else prepare_tile)(region,errors[i],rects[i],width) for i in slots];tiles=[t for t,_ in prepared];size=sum(b for _,b in prepared)
 if not tiles:
  return dict(points=[],faces=[],roles=[],changedFaces=[],patches=[],preservedCore=[],loadedPoolIds=[],tileBounds=rects,activeBounds=[],tileSlots=[],stats=dict(changedTriangles=0,unchangedTriangles=0,haloLayers=layers,runtimeWidth=0,bodyFaces=0,haloFaces=0,refined=refined or bool(layers),candidates=candidates and not refined and not layers,vertices=0,triangles=0,coreFaces=0,retainedCoreFaces=0,patchTriangles=0,extraVertices=0,qualityAdditions=0,minimumAngle=0,sharpTriangles=0,runtimeMs=0,processMs=0,loadedBytes=0,loadedVertices=0,loadedTriangles=0,triangulationVertices=0,excludedCoreVertices=0,qualitySatisfied=True,tileCount=0,width=width,local=local))
 # 実行時へ渡すのは保存済みタイルだけ。最詳細TINは渡さない。
 same_mesh=len({t["meshKey"] for t in tiles})==1
 if layers:
  from overlap_layers import reach
  runtime_width=max(1.,max(reach(t) for t in tiles))
 else:runtime_width=width
 runtime_tiles=[dict(t,support=[]) for t in tiles] if same_mesh or not candidates else tiles
 req=json.dumps(dict(tiles=runtime_tiles,width=runtime_width,local=local,partial=len(slots)!=len(rects))).encode();start=time.perf_counter()
 result=subprocess.run([str(ROOT/'build/terrain-q-cache-target/release/terrain-overlap-compare.exe')],input=req,capture_output=True,check=True,creationflags=subprocess.CREATE_NO_WINDOW)
 body=json.loads(result.stdout);mark_changed_faces(body,tiles);body['stats'].update(haloLayers=layers,runtimeWidth=runtime_width,bodyFaces=sum(t.get("bodyFaces",0) for t in tiles),haloFaces=sum(t.get("haloFaces",0) for t in tiles),refined=refined or bool(layers),candidates=candidates and not refined and not layers,loadedBytes=(size if candidates else sum(len(json.dumps(dict(t,support=[]),separators=(",",":")).encode()) for t in tiles)),processMs=(time.perf_counter()-start)*1000,width=width,tileCount=len(tiles));body['tileBounds']=rects;body['activeBounds']=[rects[i] for i in slots];body['tileSlots']=slots
 with LOCK,sqlite3.connect(OUT/'results.sqlite3') as db:
  table=('results_local' if local else 'results')+(f'_layers{layers}' if layers else '_refined' if refined else '' if candidates else '_no_support')
  db.execute(f'create table if not exists {table}(region text,errors text,layout text,axis integer,width real,body blob,primary key(region,errors,layout,axis,width))')
  db.execute(f'insert or replace into {table} values(?,?,?,?,?,?)',(region,json.dumps(errors),layout,axis,width,result.stdout))
 return body
class Handler(SimpleHTTPRequestHandler):
 def do_GET(self):
  u=urlsplit(self.path)
  try:
   if u.path=='/mesh':
    q=parse_qs(u.query);layout=q.get('layout',['dual'])[0];n=len(bounds(layout,int(q.get('axis',['0'])[0])));errors=[(None if q.get(k,[d])[0]=='none' else float(q.get(k,[d])[0])) for k,d in [('fine','50'),('coarse','100'),('third','200'),('fourth','400')]][:n]
    body=json.dumps(make(q.get('region',['fuji'])[0],errors,layout,int(q.get('axis',['0'])[0]),int(q.get('width',['1000'])[0]),local=q.get('mode',['local'])[0]!='global',candidates=q.get('candidates',['1'])[0]!='0',refined=q.get('refined',['0'])[0]=='1',layers=int(q.get('layers',['0'])[0]))).encode();mime='application/json'
   else:
    name={'/':'overlap_compare.html','/overlap_compare.js':'overlap_compare.js'}.get(u.path)
    if not name:return self.send_error(404)
    body=Path(__file__).with_name(name).read_bytes();mime='text/html; charset=utf-8' if name.endswith('html') else 'text/javascript; charset=utf-8'
   self.send_response(200);self.send_header('Content-Type',mime);self.send_header('Cache-Control','no-store');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
  except Exception as e:self.send_error(400,str(e))
 def log_message(self,*args):pass
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--port',type=int,default=61458);args=p.parse_args();ThreadingHTTPServer(('127.0.0.1',args.port),Handler).serve_forever()
