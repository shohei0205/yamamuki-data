"""地域DEMの用意とRust製LOD比較データの読み取り専用配信。"""
import argparse,json,sqlite3,struct
from pathlib import Path
from http.server import SimpleHTTPRequestHandler,ThreadingHTTPServer
from urllib.parse import urlsplit
ROOT=Path(__file__).resolve().parents[2]
OUT=ROOT/'build/decimation-compare-20261023'
REGIONS={'fuji':(35.3606,138.7274),'yari':(36.3419,137.6476),'tsukuba':(36.2254,140.1067)}
def prepare():
 from fixed_mesh_preview import source_grid
 OUT.mkdir(exist_ok=True)
 with sqlite3.connect((ROOT/'build/terrain-lod.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as db:
  for name,(lat,lon) in REGIONS.items():
   grid,_=source_grid(db,lat,lon,extent=16000,size=1601)
   # 表示のY軸を北向きに合わせる。生成はRustで行う。
   grid[::-1].astype('<f8').tofile(OUT/(name+'.f64'))
   print(name,grid.shape,flush=True)
class Handler(SimpleHTTPRequestHandler):
 def end_headers(self):
  self.send_header('Cache-Control','no-store');super().end_headers()
 def send(self,body,mime):
  self.send_response(200);self.send_header('Content-Type',mime);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def do_GET(self):
  path=urlsplit(self.path).path
  if path=='/report':
   with sqlite3.connect((OUT/'comparison.sqlite3').as_uri()+'?mode=ro',uri=True) as db:
    rows=[dict(region=r,method=m,**json.loads(s)) for r,m,s in db.execute('select region,method,report from meshes order by region,error,method')]
   for row in rows:row.pop('sourceIds',None)
   self.send(json.dumps(rows,ensure_ascii=False).encode(),'application/json; charset=utf-8');return
  if path.startswith('/mesh/'):
   try:
    region,method,error=path.removeprefix('/mesh/').split('/')
    with sqlite3.connect((OUT/'comparison.sqlite3').as_uri()+'?mode=ro',uri=True) as db:
     row=db.execute('select points,indices,report from meshes where region=? and method=? and error=?',(region,method,float(error))).fetchone()
    if not row:return self.send_error(404)
    p,f,s=row;s=json.loads(s);self.send(struct.pack('<4I',s['vertices'],s['triangles'],len(p),len(f))+p+f,'application/octet-stream');return
   except (ValueError,sqlite3.Error):return self.send_error(400)
  if path=='/triangle_frontier_decoder.wasm':
   self.send((ROOT/'build/hierarchical-lod-20261023/triangle_frontier_decoder.wasm').read_bytes(),'application/wasm');return
  files={'/':'decimation_compare.html','/decimation_compare.js':'decimation_compare.js','/triangle_frontier_wasm.mjs':'triangle_frontier_wasm.mjs'}
  if path not in files:return self.send_error(404)
  f=Path(__file__).with_name(files[path]);self.send(f.read_bytes(),'text/html; charset=utf-8' if f.suffix=='.html' else 'text/javascript; charset=utf-8')
 def log_message(self,*args):pass
if __name__=='__main__':
 parser=argparse.ArgumentParser();parser.add_argument('--prepare',action='store_true');parser.add_argument('--port',type=int,default=61456);args=parser.parse_args()
 if args.prepare:prepare()
 else:ThreadingHTTPServer(('127.0.0.1',args.port),Handler).serve_forever()
