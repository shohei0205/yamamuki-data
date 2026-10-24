"""異なるLODを接合する比較試験。接合帯は細メッシュ側の別データとして保存する。"""
import argparse,json,sqlite3,math,threading,subprocess,time
from pathlib import Path
from http.server import SimpleHTTPRequestHandler,ThreadingHTTPServer
from urllib.parse import urlsplit,parse_qs
ROOT=Path(__file__).resolve().parents[2]
OUT=ROOT/'build/decimation-compare-20261023'
LOCK=threading.Lock()

def clip(poly,axis,side):
 out=[]
 for a,b in zip(poly,poly[1:]+poly[:1]):
  ia=side*(a[axis]-8000)>=-1e-8;ib=side*(b[axis]-8000)>=-1e-8
  if ia:out.append(a)
  if ia!=ib:
   t=(8000-a[axis])/(b[axis]-a[axis]);p=[a[k]+t*(b[k]-a[k]) for k in range(3)];p[axis]=8000.;out.append(p)
 unique=[]
 for p in out:
  if not unique or math.dist(p[:2],unique[-1][:2])>1e-7:unique.append(p)
 if len(unique)>1 and math.dist(unique[0][:2],unique[-1][:2])<1e-7:unique.pop()
 return unique if len(unique)>=3 else []

def build(fine,coarse,axis=0,enabled=True):
 # 左・下を細メッシュ、右・上を粗メッシュとし、同じ地理範囲を切り分ける。
 other=1-axis;parts=[];seam={};fine_seam={}
 for role,mesh,side in [('fine',fine,-1),('coarse',coarse,1)]:
  cells=[]
  for fid,f in enumerate(mesh['faces']):
   poly=clip([mesh['points'][v] for v in f],axis,side)
   if not poly:continue
   cells.append((fid,poly))
   for p in poly:
    if abs(p[axis]-8000)<1e-7:
     key=round(p[other],6);seam[key]=p[other]
     if role=='fine':fine_seam[key]=p[2]
  parts.append((role,cells))
 knots=sorted(fine_seam)
 def z(t):
  import bisect
  i=max(0,min(len(knots)-2,bisect.bisect_right(knots,t)-1));a,b=knots[i:i+2]
  return fine_seam[a]+(fine_seam[b]-fine_seam[a])*(t-a)/(b-a)
 points=[];faces=[];roles=[];lookup={};removed=[];patch_faces=[];extra=[]
 def vertex(p):
  key=tuple(round(v,7) for v in p)
  if key not in lookup:lookup[key]=len(points);points.append(list(key))
  return lookup[key]
 for role,cells in parts:
  for fid,poly in cells:
   touched=sum(abs(p[axis]-8000)<1e-7 for p in poly)>=2
   if enabled and touched:
    expanded=[]
    for a,b in zip(poly,poly[1:]+poly[:1]):
     a=list(a);b=list(b)
     if abs(a[axis]-8000)<1e-7:a[other]=round(a[other],6);a[2]=z(a[other])
     expanded.append(a)
     if abs(a[axis]-8000)<1e-7 and abs(b[axis]-8000)<1e-7:
      lo,hi=sorted((a[other],b[other]));ts=[t for t in seam if lo+1e-7<t<hi-1e-7]
      for t in sorted(ts,reverse=b[other]<a[other]):
       p=[0.,0.,z(t)];p[axis]=8000.;p[other]=t;expanded.append(p)
    poly=expanded
    # 境界の全分割点を残すため、凸多角形の重心から扇状に分割する。
    center=[sum(p[k] for p in poly)/len(poly) for k in range(3)]
    # 粗い側の内点は元の粗い平面高さを保つ。境界だけ細い側へ接続する。
    if role=='coarse':
     original=coarse['faces'][fid];a,b,c=[coarse['points'][i] for i in original];dx,dy=center[0]-a[0],center[1]-a[1];det=(b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0]);u=(dx*(c[1]-a[1])-dy*(c[0]-a[0]))/det;v=((b[0]-a[0])*dy-(b[1]-a[1])*dx)/det;center[2]=a[2]+u*(b[2]-a[2])+v*(c[2]-a[2]);removed.append(fid)
    ci=vertex(center);ids=[vertex(p) for p in poly]
    tris=[[ci,ids[i],ids[(i+1)%len(ids)]] for i in range(len(ids))]
   else:
    ids=[vertex(p) for p in poly];tris=[[ids[0],ids[i],ids[i+1]] for i in range(1,len(ids)-1)]
   for tri in tris:
    a,b,c=[points[i] for i in tri];cross=(b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])
    if cross<1e-6:continue
    if enabled and touched and role=='coarse':patch_faces.append(len(faces))
    faces.append(tri);roles.append(2 if enabled and touched and role=='coarse' else 0 if role=='fine' else 1)
 seamids=[i for i,p in enumerate(points) if abs(p[axis]-8000)<1e-7]
 patchids=sorted(set(v for i in patch_faces for v in faces[i]));coarsepoints={tuple(round(v,7) for v in p) for p in coarse['points']}
 extra=[i for i in patchids if tuple(points[i]) not in coarsepoints]
 patchmap={v:i for i,v in enumerate(patchids)}
 return dict(points=points,faces=faces,roles=roles,seam=seamids,patch=dict(owner='fine',points=[points[i] for i in patchids],faces=[[patchmap[v] for v in faces[i]] for i in patch_faces],removedCoarseFaces=removed,faceIds=patch_faces,vertexIds=patchids,extraVertexIds=extra),stats=dict(vertices=len(points),triangles=len(faces),patchTriangles=len(patch_faces),extraVertices=len(extra)))

def make(region,fine,coarse,axis,enabled):
 assert region in ('fuji','yari','tsukuba') and fine in (12.5,25,50,100,200,400,800) and coarse in (12.5,25,50,100,200,400,800) and fine<=coarse
 with sqlite3.connect((OUT/'comparison.sqlite3').as_uri()+'?mode=ro',uri=True) as db:
  meshes=[json.loads(db.execute("select preview from meshes where region=? and method='decimated-boundary' and error=?",(region,e)).fetchone()[0]) for e in (fine,coarse)]
 body=build(*meshes,axis,enabled)
 with LOCK,sqlite3.connect(OUT/'seam-patches.sqlite3') as db:
  db.execute('create table if not exists patches(region text,fine real,coarse real,axis integer,enabled integer,body text,primary key(region,fine,coarse,axis,enabled))')
  db.execute('insert or replace into patches values(?,?,?,?,?,?)',(region,fine,coarse,axis,int(enabled),json.dumps(body,separators=(',',':'))))
 return body
def make_rust(region,errors,axis=0,enabled=True,quad=False,shared=False):
 assert region in ('fuji','yari','tsukuba') and axis in (0,1) and len(errors)==(4 if quad else 2)
 assert all(e in (12.5,25,50,100,200,400,800) for e in errors)
 with sqlite3.connect((OUT/'comparison.sqlite3').as_uri()+'?mode=ro',uri=True) as db:
  meshes=[json.loads(db.execute("select preview from meshes where region=? and method='decimated-boundary' and error=?",(region,e)).fetchone()[0]) for e in errors]
 request=dict(meshes=meshes,errors=errors,axis=axis,enabled=enabled,quad=quad,shared=shared and enabled)
 if shared:
  with sqlite3.connect((OUT/'comparison.sqlite3').as_uri()+'?mode=ro',uri=True) as db:
   request['detail']=json.loads(db.execute("select preview from meshes where region=? and method='decimated-boundary' and error=12.5",(region,)).fetchone()[0])
 start=time.perf_counter()
 result=subprocess.run([str(ROOT/'build/terrain-q-cache-target/release/terrain-seam-compare.exe')],input=json.dumps(request).encode(),capture_output=True,check=True,creationflags=subprocess.CREATE_NO_WINDOW)
 body=json.loads(result.stdout)
 if shared:body['reference']=dict(region=region,method='decimated-boundary',error=12.5)
 body['stats']['generationMs']=(time.perf_counter()-start)*1000
 with LOCK,sqlite3.connect(OUT/'seam-patches.sqlite3') as db:
  table='rust_shared_patches' if shared else 'rust_patches'
  db.execute(f'create table if not exists {table}(region text,errors text,axis integer,enabled integer,quad integer,body text,primary key(region,errors,axis,enabled,quad))')
  db.execute(f'insert or replace into {table} values(?,?,?,?,?,?)',(region,json.dumps(errors),axis,int(enabled),int(quad),json.dumps(body,separators=(',',':'))))
 return body
class Handler(SimpleHTTPRequestHandler):
 def do_GET(self):
  u=urlsplit(self.path)
  try:
   if u.path=='/mesh':
    q=parse_qs(u.query);region=q['region'][0];fine=float(q['fine'][0]);coarse=float(q['coarse'][0]);axis=int(q['axis'][0]);enabled=q.get('join',['1'])[0]=='1'
    if q.get('engine',['rust'])[0]=='python':data=make(region,fine,coarse,axis,enabled)
    else:
     quad=q.get('layout',['dual'])[0]=='quad';errors=[fine,coarse]+([float(q.get('third',['200'])[0]),float(q.get('fourth',['400'])[0])] if quad else [])
     data=make_rust(region,errors,axis,enabled,quad,shared=q.get('engine',['rust-shared'])[0]=='rust-shared')
    body=json.dumps(data).encode();mime='application/json'
   else:
    name={'/':'seam_compare.html','/seam_compare.js':'seam_compare.js'}.get(u.path)
    if not name:return self.send_error(404)
    body=Path(__file__).with_name(name).read_bytes();mime='text/html; charset=utf-8' if name.endswith('html') else 'text/javascript; charset=utf-8'
   self.send_response(200);self.send_header('Content-Type',mime);self.send_header('Cache-Control','no-store');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
  except Exception as e:self.send_error(400,str(e))
 def log_message(self,*args):pass
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--port',type=int,default=61457);args=p.parse_args();ThreadingHTTPServer(('127.0.0.1',args.port),Handler).serve_forever()
