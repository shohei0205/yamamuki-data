"""比較用メッシュBLOBと静的ページだけを中継する。"""
import argparse,json,re,sqlite3
from functools import partial
from http.server import SimpleHTTPRequestHandler,ThreadingHTTPServer
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
class Handler(SimpleHTTPRequestHandler):
    database=ROOT/'build/progressive-tin-compare.sqlite3'
    def end_headers(self):
        self.send_header('Cache-Control', 'no-store')
        super().end_headers()

    def do_GET(self):
        path=self.path.split('?')[0]
        if path=='/':self.path='/progressive.html';return super().do_GET()
        separated=re.fullmatch(r'/split/(fuji|yari|plain)/(independent|shared|history|reference|coarse_independent|coarse_shared|coarse_history|opt_[a-z_]+)/(\d+|-1)/(vertices|normals|indices|base|delta|mesh)',path)
        if separated or path=='/manifest':
            with sqlite3.connect((ROOT/'build/progressive-split.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as db:
                row=db.execute('SELECT data FROM blobs WHERE region=? AND format=? AND level=? AND kind=?',(separated[1],separated[2],int(separated[3]),separated[4])).fetchone() if separated else db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()
            if not row:return self.send_error(404)
            body=row[0] if separated else row[0].encode('utf-8');self.send_response(200);self.send_header('Content-Type','application/octet-stream' if separated else 'application/json');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
        hint=re.fullmatch(r'/curve-hints/(fuji|yari|plain)/(0|1|2|3|4|5)',path)
        if hint:
            with sqlite3.connect((ROOT/'build/curve-hints.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as db:row=db.execute('SELECT data FROM hints WHERE region=? AND level=?',(hint[1],int(hint[2]))).fetchone()
            if not row:return self.send_error(404)
            body=row[0];self.send_response(200);self.send_header('Content-Type','application/octet-stream');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
        normal=re.fullmatch(r'/(normal|normal-filtered|normal-lpf)/(fuji|yari|plain)/(128|256|512|685)\.png',path)
        if normal:
            suffix={'normal':'','normal-filtered':'-filtered','normal-lpf':'-lpf'}[normal[1]]
            file=ROOT/f'build/progressive-normal-maps/{normal[2]}-{normal[3]}{suffix}.png'
            if not file.exists():return self.send_error(404)
            body=file.read_bytes();self.send_response(200);self.send_header('Content-Type','image/png');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
        optimized=re.fullmatch(r'/optimized/(fuji|yari|plain)/(z_rms|z_max|xy_rms|xy_max|feature_z|feature_xy|volume_z|volume_xy|original)/(0|1|2)',path)
        if optimized:
            database=ROOT/'build/tin-optimized-compare.sqlite3'
            if not database.exists():return self.send_error(404)
            with sqlite3.connect(database.resolve().as_uri()+'?mode=ro',uri=True) as db:row=db.execute('SELECT data FROM meshes WHERE region=? AND variant=? AND level=?',(optimized[1],optimized[2],int(optimized[3]))).fetchone()
            if not row:return self.send_error(404)
            body=row[0];self.send_response(200);self.send_header('Content-Type','application/octet-stream');self.send_header('Content-Length',str(len(body)));self.send_header('Cache-Control','no-store');self.end_headers();self.wfile.write(body);return
        match=re.fullmatch(r'/blob/(fuji|yari|plain)/(independent|shared|history|reference)/(-1|0|1|2)/(vertices|indices|base|delta)',path)
        if path=='/manifest' or match:
            database=ROOT/'build/progressive-tin-reference.sqlite3' if match and match[2]=='reference' else self.database
            with sqlite3.connect(database.resolve().as_uri()+'?mode=ro',uri=True) as db:
                row=db.execute('SELECT data FROM blobs WHERE region=? AND format=? AND level=? AND kind=?',(match[1],'independent' if match[2]=='reference' else match[2],int(match[3]),match[4])).fetchone() if match else db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()
            if not row:return self.send_error(404)
            body=row[0] if match else row[0].encode('utf-8')
            reference_db=ROOT/'build/progressive-tin-reference.sqlite3'
            if not match and reference_db.exists():
                with sqlite3.connect(reference_db.resolve().as_uri()+'?mode=ro',uri=True) as db:ref=db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()
                if ref:
                    meta=json.loads(body);meta['reference']=json.loads(ref[0]);body=json.dumps(meta,ensure_ascii=False).encode('utf-8')
            optimized_db=ROOT/'build/tin-optimized-compare.sqlite3'
            if not match and optimized_db.exists():
                with sqlite3.connect(optimized_db.resolve().as_uri()+'?mode=ro',uri=True) as db:opt=db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()
                if opt:
                    meta=json.loads(body);meta['optimization']=json.loads(opt[0]);body=json.dumps(meta,ensure_ascii=False).encode('utf-8')
            self.send_response(200);self.send_header('Content-Type','application/octet-stream' if match else 'application/json; charset=utf-8');self.send_header('Content-Length',str(len(body)));self.send_header('Cache-Control','no-store');self.end_headers();self.wfile.write(body);return
        if path not in ('/progressive.html','/progressive.js','/progressive_worker.js','/pn-terrain.js','/vendor/meshoptimizer/meshopt_simplifier.js','/vendor/meshoptimizer/LICENSE.md'):return self.send_error(404)
        super().do_GET()
if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--port',type=int,default=61444);args=parser.parse_args();ThreadingHTTPServer(('127.0.0.1',args.port),partial(Handler,directory=str(Path(__file__).parent))).serve_forever()
