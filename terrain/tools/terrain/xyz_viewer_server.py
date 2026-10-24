"""新形式の地域メッシュをBLOBのままブラウザーへ渡す試験サーバー。"""
import argparse, json, sqlite3, struct
from pathlib import Path
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs
ROOT = Path(__file__).resolve().parents[2]
REGIONS = {"tsukuba", "yari", "fuji"}
class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        if urlparse(self.path).path!="/api/bundle":self.send_error(404);return
        try:
            length=int(self.headers.get("Content-Length","0"))
            if not 0<=length<=4000000:raise ValueError("キャッシュ一覧が大きすぎます")
            entries=json.loads(self.rfile.read(length)).get("known",[])
            self.known={tuple(v[:3]):(bool(v[3]),bool(v[4])) for v in entries if len(v)==5}
        except (ValueError,TypeError):self.send_error(400);return
        self.do_GET()
    def do_GET(self):
        try:
            url=urlparse(self.path); q=parse_qs(url.query)
            if url.path in ("/", "/xyz_viewer.js", "/xyz_wasm.mjs", "/xyz_decoder.wasm"):
                name="xyz_viewer.html" if url.path=="/" else url.path[1:]
                data=(ROOT/"build/xyz_decoder.wasm" if name.endswith(".wasm") else Path(__file__).with_name(name)).read_bytes()
                mime="application/wasm" if name.endswith(".wasm") else "text/html; charset=utf-8" if name.endswith("html") else "text/javascript; charset=utf-8"
            elif url.path in ("/api/meta", "/api/tiles", "/api/tile", "/api/bundle"):
                region=q.get("region",["fuji"])[0]
                if region not in REGIONS: raise ValueError("地域が不正です")
                directory=self.server.output/region
                with sqlite3.connect((directory/"terrain.sqlite3").as_uri()+"?mode=ro",uri=True) as db:
                    if url.path=="/api/meta":
                        value=json.loads((directory/"region.json").read_text())
                        stat=(directory/"terrain.sqlite3").stat()
                        value["identity"]=str(stat.st_size)+":"+str(stat.st_mtime_ns)
                        value["levels"]=[r[0] for r in db.execute("select distinct z from tiles order by z")]
                        data=json.dumps(value).encode();mime="application/json"
                    elif url.path in ("/api/tiles", "/api/bundle"):
                        z=int(q["z"][0]);span=float(q.get("span",["32000"])[0])
                        if not 6<=z<=14 or not 1000<=span<=250000:raise ValueError("範囲が不正です")
                        r=json.loads((directory/"region.json").read_text())
                        world=40075016.68557849
                        cx=(r["px"]+(r["n"]-1)/2)/4194304
                        cy=(r["py"]+(r["n"]-1)/2)/4194304
                        x0=int((cx-span/2/world)*(1<<z));x1=int((cx+span/2/world)*(1<<z))
                        y0=int((cy-span/2/world)*(1<<z));y1=int((cy+span/2/world)*(1<<z))
                        rows=list(db.execute("select tile_x,tile_y,vertex_count,body_triangle_count,length(vertices)+length(triangles) from tiles where z=? and tile_x between ? and ? and tile_y between ? and ? order by tile_y,tile_x",(z,x0,x1,y0,y1)))
                        if url.path=="/api/tiles":
                            data=json.dumps(rows).encode();mime="application/json"
                        else:
                            requested={(z,r[0],r[1]) for r in rows}
                            pending=list(requested); blobs={}
                            while pending:
                                key=pending.pop()
                                if key in blobs:continue
                                row=db.execute("select vertices,triangles from tiles where z=? and tile_x=? and tile_y=?",key).fetchone()
                                if row is None:raise ValueError("親タイルがありません")
                                vb,tb=row;tb=tb if key in requested else b""
                                blobs[key]=(vb,tb)
                                deps=struct.unpack_from("<I",vb,12)[0]
                                for i in range(deps):pending.append(struct.unpack_from("<Iii",vb,32+i*12))
                            entries=[];parts=[]
                            known=getattr(self,"known",{})
                            for key,(vb,tb) in sorted(blobs.items()):
                                has_vertices,has_triangles=known.get(key,(False,False))
                                if has_vertices:vb=b""
                                if has_triangles:tb=b""
                                if not vb and not tb:continue
                                entries.append([*key,len(vb),len(tb)]);parts.extend((vb,tb))
                            manifest=json.dumps(dict(rows=rows,entries=entries)).encode()
                            data=b"".join([struct.pack("<I",len(manifest)),manifest,*parts]);mime="application/octet-stream"
                    else:
                        z,x,y=(int(q[k][0]) for k in ("z","x","y"))
                        row=db.execute("select vertices,triangles from tiles where z=? and tile_x=? and tile_y=?",(z,x,y)).fetchone()
                        if row is None:self.send_error(404);return
                        a,b=row;data=struct.pack("<II",len(a),len(b))+a+b;mime="application/octet-stream"
            else:self.send_error(404);return
            self.send_response(200);self.send_header("Content-Type",mime);self.send_header("Cache-Control","no-store");self.send_header("Content-Length",str(len(data)));self.end_headers();self.wfile.write(data)
        except (ValueError,KeyError,sqlite3.Error,OSError) as e:
            self.send_error(400,str(e))
    def log_message(self,*args):pass
if __name__=="__main__":
    p=argparse.ArgumentParser();p.add_argument("--port",type=int,default=61461);p.add_argument("--output",type=Path,default=ROOT/"build/xyz-regional-200km-local-fast-20261024");a=p.parse_args()
    server=ThreadingHTTPServer(("127.0.0.1",a.port),Handler);server.output=a.output.resolve();server.serve_forever()
