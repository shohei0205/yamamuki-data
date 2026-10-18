"""比較用の PNG と SQLite の地形 BLOB を加工せず中継する。"""
import argparse
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import sqlite3
import struct
import re
from contextlib import closing
from urllib.parse import parse_qs, urlsplit
from sqlite_relay import pack_rows

ROOT = Path(__file__).resolve().parents[2]


class Handler(SimpleHTTPRequestHandler):
    def do_POST(self):
        if self.path not in ('/dynamic-jxl/encode','/national-sun/start'):
            self.send_error(404);return
        try:
            from dynamic_jxl import encode
            if self.headers.get('Content-Type','').split(';')[0] != 'application/json':
                raise ValueError('JSON で指定してください')
            length=int(self.headers.get('Content-Length','0'))
            if not 0 < length <= 2048:raise ValueError('リクエストの長さが不正です')
            options=json.loads(self.rfile.read(length))
            if self.path == '/national-sun/start':
                from national_sun import start
                result=start(options)
            else:
                result=encode(options)
            status=200
        except Exception as e:
            result={'error':str(e)};status=409 if isinstance(e,BlockingIOError) else 400
        body=json.dumps(result,ensure_ascii=False).encode('utf-8')
        self.send_response(status);self.send_header('Content-Type','application/json; charset=utf-8');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)

    def do_GET(self):
        route = self.path.split('?')[0]
        if route=='/national-height':
            self.path='/national_height.html';super().do_GET();return
        if route in ('/terrain-lod.js','/height-lod-edges.js','/sqlite-height-client.js','/sqlite-height-worker.js','/national_height.html','/national_height.js','/national-navigation.js','/height-boundaries.js','/height-mesh.js','/ground-camera.js'):
            super().do_GET();return
        if route=='/national-height-data/manifest.json' or re.fullmatch(r'/national-height-data/(?:[7-9]|1[0-2])/[0-9]{1,4}/[0-9]{1,4}[.](?:png|mask)',route):
            path=ROOT/'build/terrain-height-browser'/route[len('/national-height-data/'):]
            if not path.is_file():self.send_error(404);return
            body=path.read_bytes();self.send_response(200);self.send_header('Content-Type','application/json' if path.suffix=='.json' else 'image/png' if path.suffix=='.png' else 'application/octet-stream');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
        if route=='/water-tiles':
            try:
                from water_tile_relay import relay
                body=relay(ROOT/'build/terrain-inland-vectors/source.sqlite3',parse_qs(urlsplit(self.path).query));code=200
            except Exception as e:
                body=json.dumps({'error':str(e)},ensure_ascii=False).encode('utf-8');code=400
            self.send_response(code);self.send_header('Content-Type','application/octet-stream' if code==200 else 'application/json');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
        if re.fullmatch(r'/inland-source/(?:NOTICE[.]txt|ODbL-1[.]0[.]txt|rivers[.]geojson[.]gz|water[.]geojson[.]gz)',route):
            path=ROOT/'build/terrain-inland-vectors'/('source-'+route.rsplit('/',1)[1])
            if not path.is_file():self.send_error(404);return
            body=path.read_bytes();self.send_response(200);self.send_header('Content-Type','text/plain; charset=utf-8' if path.suffix=='.txt' else 'application/gzip');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
        if re.fullmatch(r'/inland-vectors/(?:national|(?:native|region)-(?:yari|tsukuba)-L(?:[4-9]|1[0-4]))[.]inland',route):
            path=ROOT/'build/terrain-inland-vectors'/route.rsplit('/',1)[1]
            if not path.is_file():self.send_error(404);return
            body=path.read_bytes();self.send_response(200);self.send_header('Content-Type','application/octet-stream');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
        if re.fullmatch(r'/sea-vectors/(?:national|(?:native|region)-(?:yari|tsukuba)-L(?:[4-9]|1[0-4]))[.]sea',route):
            path=ROOT/'build/terrain-sea-vectors'/route.rsplit('/',1)[1]
            if not path.is_file():self.send_error(404);return
            body=path.read_bytes();self.send_response(200);self.send_header('Content-Type','application/octet-stream');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
        native = re.fullmatch(r'/tile-regions/(yari|tsukuba)/(levels.json|levels/(?:[4-9]|1[0-4])/(?:manifest.json|height.png|normals/(?:512|1024).png))',route)
        if native:
            path=ROOT/f'build/terrain-tile-preview-{native[1]}'/native[2]
            if not path.is_file():self.send_error(404);return
            body=path.read_bytes();self.send_response(200);self.send_header('Content-Type','application/json' if path.suffix=='.json' else 'image/png');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
        if route == '/national-geometry/levels.json' or re.fullmatch(r'/national-geometry/height-L[4-7].f32',route):
            compressed=not route.endswith('.json')
            path=ROOT/'build/terrain-national-shading'/('geometry-levels.json' if not compressed else route.rsplit('/',1)[1]+'.gz')
            if not path.is_file():self.send_error(404);return
            body=path.read_bytes();self.send_response(200);self.send_header('Content-Type','application/octet-stream' if compressed else 'application/json');self.send_header('Content-Length',str(len(body)))
            if compressed:self.send_header('Content-Encoding','gzip')
            self.end_headers();self.wfile.write(body);return
        if route == '/national-sun/status':
            try:
                from national_sun import status
                query=parse_qs(urlsplit(self.path).query)
                result=status({'datetime':query['datetime'][0]} if 'datetime' in query else {'azimuth':int(query['azimuth'][0]),'altitude':int(query['altitude'][0])});code=200
            except Exception as e:
                result={'error':str(e)};code=400
            body=json.dumps(result,ensure_ascii=False).encode('utf-8');self.send_response(code);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
        if re.fullmatch(r'/national-sun/(?:a[0-9]{1,3}-h[0-9]{1,2}|t[0-9]{8}T[0-9]{4})/shade(?:-L[4-7])?.png',route):
            path=ROOT/'build/terrain-national-sun'/route.split('/')[2]/route.rsplit('/',1)[1]
            if not path.is_file():self.send_error(404);return
            body=path.read_bytes();self.send_response(200);self.send_header('Content-Type','image/png');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
        if re.fullmatch(r'/national-shading-data/(?:manifest.json|progress.json|illumination-levels.json|height.f32|normal-detail.png|normal-equal.png|normal-L[4-7].png|shade-L[4-7].png|shade.png|coverage.png)',route):
            path=ROOT/'build/terrain-national-shading'/route.rsplit('/',1)[1]
            if not path.is_file():self.send_error(404);return
            body=path.read_bytes();self.send_response(200);self.send_header('Content-Type','application/json' if path.suffix=='.json' else 'image/png' if path.suffix=='.png' else 'application/octet-stream');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
        if route == '/national-shading':
            self.path='/national_shading.html';super().do_GET();return
        if route == '/national_shading.js':
            super().do_GET();return
        regional = re.fullmatch(r'/regions/(yari|tsukuba)/(levels.json|levels/(?:[4-9]|1[0-4])/(?:manifest.json|height.png|normal.png|normal-coarse.png)|normals/(?:4|8|16|32|64|128|256|512|1024|2048).png)', route)
        if regional:
            path=ROOT/f'build/terrain-heightmap-{regional[1]}'/regional[2]
            if not path.is_file():self.send_error(404);return
            body=path.read_bytes();self.send_response(200);self.send_header('Content-Type','application/json' if path.suffix=='.json' else 'image/png');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
        if re.fullmatch(r'/dynamic-jxl/[a-f0-9]{32}/(?:height[.]f32|normal[.]png|result[.]json)',route):
            path=ROOT/'build/terrain-jxl-dynamic'/route[len('/dynamic-jxl/'):]
            if not path.is_file():self.send_error(404);return
            body=path.read_bytes();self.send_response(200);self.send_header('Content-Type','image/png' if path.suffix=='.png' else 'application/octet-stream');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
        elif route == '/favicon.ico':
            self.send_response(204);self.end_headers()
        elif route == '/codecs/report':
            report={}
            for name in ('normals','meshes','heights','status'):
                path=ROOT/'build/terrain-codec-comparison'/f'{name}.json'
                if not path.exists():path=path.with_name(f'{name}.partial.json')
                if path.exists():
                    try:report[name]=json.loads(path.read_text('utf-8'))
                    except json.JSONDecodeError:pass
            body=json.dumps(report,ensure_ascii=False).encode('utf-8')
            self.send_response(200);self.send_header('Content-Type','application/json; charset=utf-8');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
        elif re.fullmatch(r'/codecs/(normals|meshes|heights)/[a-zA-Z0-9_.-]+',route):
            path=ROOT/'build/terrain-codec-comparison'/route[len('/codecs/'):]
            if not path.exists():self.send_error(404);return
            body=path.read_bytes();self.send_response(200);self.send_header('Content-Type','image/png' if path.suffix=='.png' else 'application/octet-stream');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
        elif route == '/jpeg-xl/report.json':
            path=ROOT/'build/terrain-codec-comparison/jpeg-xl/report.json'
            if not path.is_file():self.send_error(404);return
            body=path.read_bytes();self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
        elif route == '/height-astc/report.json' or re.fullmatch(r'/height-astc/L(?:[4-9]|1[0-4])-astc(?:8|12)(?:[.]yha|-decoded[.]png)',route):
            path=ROOT/'build/terrain-codec-comparison'/route[1:]
            if not path.is_file():self.send_error(404);return
            body=path.read_bytes();self.send_response(200);self.send_header('Content-Type','application/json' if path.suffix=='.json' else 'image/png' if path.suffix=='.png' else 'application/octet-stream');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
        elif route in ('/mesh-normals/report.json','/baked-normals/report.json') or re.fullmatch(r'/(?:mesh|baked)-normals/L(?:[4-9]|1[0-4])/(?:source/normals/[0-9]+[.]png|normals/[a-zA-Z0-9_.-]+)',route):
            path=ROOT/'build/terrain-codec-comparison'/route[1:]
            if not path.is_file():self.send_error(404);return
            body=path.read_bytes();self.send_response(200);self.send_header('Content-Type','application/json' if path.suffix=='.json' else 'image/png' if path.suffix=='.png' else 'application/octet-stream');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
        elif route.startswith('/codec-vendor/'):
            mapping={'meshopt_decoder.mjs':'meshoptimizer/meshopt_decoder.mjs','fzstd.js':'fzstd/umd/index.js','LercDecode.js':'lerc/LercDecode.js','lerc-wasm.wasm':'lerc/lerc-wasm.wasm'}
            name=route[len('/codec-vendor/'):]
            if name not in mapping:self.send_error(404);return
            body=(ROOT/'build/terrain-codec-tools/web'/mapping[name]).read_bytes();self.send_response(200);self.send_header('Content-Type','application/wasm' if name.endswith('.wasm') else 'text/javascript');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
        elif route in ('/manifest.json','/height.png','/normal.png','/normal-coarse.png','/levels.json') or re.fullmatch(r'/levels/(?:[4-9]|1[0-4])/(?:manifest.json|height.png|normal.png|normal-coarse.png)',route) or re.fullmatch(r'/normals/(?:4|8|16|32|64|128|256|512|1024|2048).png',route):
            path=ROOT/'build/terrain-heightmap-yari'/route[1:]
            body=path.read_bytes()
            self.send_response(200)
            self.send_header('Content-Type','application/json' if route.endswith('.json') else 'image/png')
            self.send_header('Content-Length',str(len(body)))
            self.end_headers();self.wfile.write(body)
        elif re.fullmatch(r'/polygon/(?:[4-9]|1[0-4])',route):
            level=int(route.rsplit('/',1)[1])
            database=ROOT/'build/terrain-yari-local.sqlite3'
            with closing(sqlite3.connect(database.as_uri()+'?mode=ro',uri=True)) as db:
                geometry_zoom=json.loads(db.execute("SELECT value FROM metadata WHERE key='geometryZoomByLevel'").fetchone()[0])[str(level)]
                rows=[(dict(z=z,x=x,y=y,geometryZoom=geometry_zoom,heightError=error),body) for z,x,y,body,error in db.execute('SELECT t.z,t.x,t.y,t.data,m.height_error FROM tiles t JOIN metrics m USING(z,x,y) WHERE t.z=? ORDER BY t.x,t.y',(level,))]
            body=pack_rows(rows)
            self.send_response(200);self.send_header('Content-Type','application/octet-stream')
            self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
        elif route == '/heightmap-data':
            with sqlite3.connect(self.server.database.as_uri()+'?mode=ro', uri=True) as db:
                rows = db.execute('SELECT x,y,data FROM tiles WHERE z=14 ORDER BY y,x').fetchall()
            metadata = dict(zoom=14, tileSize=256, west=min(r[0] for r in rows),
                            north=min(r[1] for r in rows), east=max(r[0] for r in rows),
                            south=max(r[1] for r in rows),
                            tiles=[dict(x=x,y=y,bytes=len(body)) for x,y,body in rows])
            header = json.dumps(metadata).encode()
            body = struct.pack('<I',len(header))+header+b''.join(r[2] for r in rows)
            self.send_response(200)
            self.send_header('Content-Type','application/octet-stream')
            self.send_header('Content-Length',str(len(body)))
            self.send_header('Cache-Control','no-store')
            self.end_headers()
            self.wfile.write(body)
        elif route in ('/','/heightmap'):
            self.path='/heightmap.html'
            super().do_GET()
        elif route == '/codec-report':
            self.path='/codec_report.html';super().do_GET()
        elif route in ('/water-tile-client.js','/inland-vectors.js','/solar-position.js','/solar-controls.js','/sea-vectors.js','/heightmap.js','/heightmap.html','/terrain-client-core.js','/terrain-codec-client.js','/codec_report.html') or route.startswith('/vendor/'):
            super().do_GET()
        else:
            self.send_error(404)

    def end_headers(self):
        self.send_header('Cache-Control','no-store')
        super().end_headers()

    def log_message(self,*args):
        pass


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--database',type=Path,default=ROOT/'build/terrain-yari-preview-source.sqlite3')
    parser.add_argument('--port',type=int,default=61440)
    args=parser.parse_args()
    server=ThreadingHTTPServer(('127.0.0.1',args.port),partial(Handler,directory=str(Path(__file__).parent)))
    server.database=args.database.resolve()
    server.serve_forever()
