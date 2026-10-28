"""新形式の地域・全国メッシュをBLOBのまま渡す試験サーバー。"""
import argparse, json, math, sqlite3, struct, threading
from xyz_selection import TileCatalog
from pathlib import Path
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs
ROOT = Path(__file__).resolve().parents[2]
REGIONS = {"tsukuba", "yari", "fuji"}
CENTERS = {"fuji": (35.3606, 138.7274), "yari": (36.3419, 137.6476), "tsukuba": (36.2254, 140.1067), "national": (36.0, 138.0)}

def region_metadata(server, region):
    if not server.national:
        return json.loads((server.output / region / "region.json").read_text())
    lat, lon = CENTERS[region]
    n = round((4000000 if region == "national" else 200000) / (40075016.68557849 / 4194304)) + 1
    cx = (lon + 180) / 360 * 4194304
    cy = (0.5 - math.log(math.tan(math.pi / 4 + math.radians(lat) / 2)) / (2 * math.pi)) * 4194304
    return dict(name=region, latitude=lat, longitude=lon, n=n, px=cx-(n-1)/2, py=cy-(n-1)/2, national=True)

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        if urlparse(self.path).path not in ("/api/bundle", "/api/scene", "/api/base"):self.send_error(404);return
        try:
            length=int(self.headers.get("Content-Length","0"))
            if not 0<=length<=4000000:raise ValueError("キャッシュ一覧が大きすぎます")
            self.payload=json.loads(self.rfile.read(length))
            entries=self.payload.get("known",[])
            self.known={tuple(v[:3]):(bool(v[3]),bool(v[4])) for v in entries if len(v)==5}
        except (ValueError,TypeError):self.send_error(400);return
        self.do_GET()
    def do_GET(self):
        try:
            url=urlparse(self.path); q=parse_qs(url.query)
            if url.path in ("/", "/xyz_viewer.js", "/xyz_wasm.mjs", "/xyz_decoder.wasm", "/xyz_terrain.js", "/xyz_scene_worker.js", "/surface-methods.js", "/loop-gpu.js"):
                name="xyz_viewer.html" if url.path=="/" else url.path[1:]
                data=(ROOT/"build/xyz_decoder.wasm" if name.endswith(".wasm") else Path(__file__).with_name(name)).read_bytes()
                if name == "xyz_viewer.html" and self.server.national:
                    data = data.decode().replace("新形式メッシュ・地域テスト", "全国メッシュ・生成結果の確認").replace("生成済み地域メッシュ", "生成済み全国メッシュ").replace('<option value="fuji">', '<option value="national">全国</option><option value="fuji">')
                    data = data.replace('<option value="200000">200 km 全域</option>', '<option value="200000">200 km</option><option value="4000000">全国 4,000 km</option>').encode()
                mime="application/wasm" if name.endswith(".wasm") else "text/html; charset=utf-8" if name.endswith("html") else "text/javascript; charset=utf-8"
            elif url.path in ("/api/meta", "/api/tiles", "/api/tile", "/api/bundle", "/api/scene", "/api/base"):
                region=q.get("region",["fuji"])[0]
                if region not in (set(CENTERS) if self.server.national else REGIONS): raise ValueError("地域が不正です")
                directory=self.server.output if self.server.national else self.server.output/region
                with sqlite3.connect((directory/"terrain.sqlite3").as_uri()+"?mode=ro",uri=True) as db:
                    if url.path=="/api/meta":
                        value=region_metadata(self.server, region)
                        stat=(directory/"terrain.sqlite3").stat()
                        value["identity"]=str(stat.st_size)+":"+str(stat.st_mtime_ns)
                        value["levels"]=[r[0] for r in db.execute("select distinct z from tiles order by z")]
                        data=json.dumps(value).encode();mime="application/json"
                    elif url.path in ("/api/tiles", "/api/bundle", "/api/scene", "/api/base"):
                        selection = None
                        if url.path == "/api/base":
                            rows=[[6,*row] for row in db.execute("SELECT tile_x,tile_y,vertex_count,triangle_count,body_triangle_count,length(vertices)+length(triangles) FROM tiles WHERE z=6 ORDER BY tile_y,tile_x")]
                            requested={tuple(row[:3]) for row in rows}
                            selection=dict(tiles=sorted(requested),levelCounts={"6":len(rows)},mode="resident",maximum=6,tileBudget=len(rows))
                        elif url.path == "/api/scene":
                            with self.server.catalog_lock:
                                catalog=self.server.catalogs.get(directory)
                                if catalog is None:
                                    catalog=TileCatalog(directory/"terrain.sqlite3")
                                    self.server.catalogs[directory]=catalog
                            selection=catalog.select(self.payload["camera"])
                            requested=set(map(tuple,selection["tiles"]))
                            rows=[[zz,x,y,*catalog.rows[zz,x,y]] for zz,x,y in selection["tiles"]]
                        else:
                            z=int(q["z"][0]);span=float(q.get("span",["32000"])[0])
                            if not 6<=z<=14 or not 1000<=span<=(5000000 if self.server.national else 250000):raise ValueError("範囲が不正です")
                            r=region_metadata(self.server, region)
                            world=40075016.68557849
                            cx=(r["px"]+(r["n"]-1)/2)/4194304
                            cy=(r["py"]+(r["n"]-1)/2)/4194304
                            x0=int((cx-span/2/world)*(1<<z));x1=int((cx+span/2/world)*(1<<z))
                            y0=int((cy-span/2/world)*(1<<z));y1=int((cy+span/2/world)*(1<<z))
                            rows=list(db.execute("select tile_x,tile_y,vertex_count,body_triangle_count,length(vertices)+length(triangles) from tiles where z=? and tile_x between ? and ? and tile_y between ? and ? order by tile_y,tile_x LIMIT 2049",(z,x0,x1,y0,y1)))
                            if len(rows)>2048:raise ValueError("表示タイルが多すぎます。表示範囲またはレベルを下げてください")
                            requested={(z,r[0],r[1]) for r in rows}
                        if url.path=="/api/tiles":
                            data=json.dumps(rows).encode();mime="application/json"
                        else:
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
                            manifest=json.dumps(dict(rows=rows,entries=entries,selection=selection)).encode()
                            data=b"".join([struct.pack("<I",len(manifest)),manifest,*parts]);mime="application/octet-stream"
                    else:
                        z,x,y=(int(q[k][0]) for k in ("z","x","y"))
                        row=db.execute("select vertices,triangles from tiles where z=? and tile_x=? and tile_y=?",(z,x,y)).fetchone()
                        if row is None:self.send_error(404);return
                        a,b=row;data=struct.pack("<II",len(a),len(b))+a+b;mime="application/octet-stream"
            else:self.send_error(404);return
            self.send_response(200);self.send_header("Content-Type",mime);self.send_header("Cache-Control","no-store");self.send_header("Content-Length",str(len(data)));self.end_headers();self.wfile.write(data)
        except (BrokenPipeError,ConnectionResetError):
            pass
        except (ValueError,KeyError,sqlite3.Error,OSError) as e:
            self.send_error(400,explain=str(e))
    def log_message(self,*args):pass
if __name__=="__main__":
    p=argparse.ArgumentParser();p.add_argument("--port",type=int,default=61461);p.add_argument("--output",type=Path,default=ROOT/"build/xyz-regional-200km-local-fast-20261024");p.add_argument("--national", action="store_true");a=p.parse_args()
    server=ThreadingHTTPServer(("127.0.0.1",a.port),Handler);server.output=a.output.resolve();server.national=a.national;server.catalogs={};server.catalog_lock=threading.Lock();server.serve_forever()
