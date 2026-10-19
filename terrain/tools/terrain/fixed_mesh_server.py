"""単一メッシュ比較の SQLite BLOB と画面だけを中継する。"""
import argparse
import json
from urllib.parse import urlsplit, parse_qs
from fixed_normal_codec import compare, correction_png, baked_texture, baked_mesh_correction
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import re
import sqlite3

ROOT = Path(__file__).resolve().parents[2]


class Handler(SimpleHTTPRequestHandler):
    database = ROOT/('build/fixed-mesh-preview-32km.sqlite3' if (ROOT/'build/fixed-mesh-preview-32km.sqlite3').exists() else 'build/fixed-mesh-preview.sqlite3')

    def normal_body(self, body):
        options=parse_qs(urlsplit(self.path).query)
        surface=options.get('surface',['normal'])[0]
        if surface not in ('normal','baked','bakedRaw','valley'):raise ValueError('表示方式が不正です')
        if surface!='normal':
            parts=urlsplit(self.path).path.split('/');region=parts[2]
            with sqlite3.connect(self.database.resolve().as_uri()+'?mode=ro',uri=True) as db:
                manifest=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
                meta=manifest['regions'][region]
                source=db.execute('SELECT data FROM blobs WHERE region=? AND kind="normal" AND variant=1024',(region,)).fetchone()[0]
                heights=db.execute('SELECT data FROM blobs WHERE region=? AND kind="height" AND variant=?',(region,meta['gridSize'])).fetchone()[0]
            columns=rows=1
            if parts[1]=='hierarchy':
                depth,x,y=map(int,parts[3:6]);width=2**depth
                resolution=manifest['hierarchy']['normalResolutions'][str(depth)];columns,rows=min(width,100-x*width),min(width,100-y*width)
            else:resolution=int(parts[-1])
            az,alt=float(options.get('az',['315'])[0]),float(options.get('alt',['40'])[0])
            import math
            if not math.isfinite(az) or not math.isfinite(alt) or not 0<=az<=360 or not 0<=alt<=90:raise ValueError('太陽の設定が不正です')
            body=baked_texture(source,heights,meta['gridSize'],resolution,'baked' if surface in ('baked','bakedRaw') else surface,az,alt,columns,rows)
            if surface=='baked':
                tolerance=int(options.get('mesh',['50'])[0])
                with sqlite3.connect(self.database.resolve().as_uri()+'?mode=ro',uri=True) as db:
                    mesh=db.execute('SELECT data FROM blobs WHERE region=? AND kind="mesh" AND variant=?',(region,tolerance)).fetchone()[0]
                body=baked_mesh_correction(body,mesh,meta['extent'],resolution,az,alt,columns,rows)
        if options.get('correction',['0'])[0]=='1' and surface=='normal':
            tolerance=int(options.get('mesh',['50'])[0]);region=urlsplit(self.path).path.split('/')[2]
            with sqlite3.connect(self.database.resolve().as_uri()+'?mode=ro',uri=True) as db:
                mesh=db.execute('SELECT data FROM blobs WHERE region=? AND kind="mesh" AND variant=?',(region,tolerance)).fetchone()[0]
                manifest=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
            parts=urlsplit(self.path).path.split('/')
            columns=rows=1
            if parts[1]=='hierarchy':
                depth,x,y=map(int,parts[3:6]);width=2**depth
                resolution=manifest['hierarchy']['normalResolutions'][str(depth)]
                columns,rows=min(width,100-x*width),min(width,100-y*width)
            else:resolution=int(parts[-1])
            cache=ROOT/'build/fixed-normal-correction'
            import hashlib
            cache.mkdir(exist_ok=True,parents=True)
            file=cache/(hashlib.sha256(b'v2'+body+mesh+str((resolution,columns,rows)).encode()).hexdigest()+'.png')
            if not file.exists():file.write_bytes(correction_png(body,mesh,manifest['regions'][region]['extent'],resolution,columns,rows))
            body=file.read_bytes()
        if 'codec' not in options:
            from io import BytesIO
            from PIL import Image
            image=Image.open(BytesIO(body))
            self.codec_report=json.dumps(dict(grayscale=image.mode=='L',storage='グレースケール1成分' if image.mode=='L' else 'RGB',codec='png',sourceOnly=True,surface=surface,width=image.width,height=image.height,bytes=len(body),sourceBytes=len(body)))
            return body
        body,report=compare(body,options['codec'][0],int(options.get('quality',['80'])[0]),float(options.get('distance',['0.3'])[0]),options.get('mode',['auto'])[0],int(options.get('block',['6'])[0]),options.get('correction',['0'])[0]=='1' or surface!='normal')
        report['surface']=surface
        self.codec_report=json.dumps(report)
        return body

    def end_headers(self):
        if hasattr(self,'codec_report'):
            self.send_header('X-Normal-Codec',self.codec_report)
            del self.codec_report
        super().end_headers()

    def do_GET(self):
        path = self.path.split('?')[0]
        if path == '/':
            self.path = '/fixed_mesh.html'
            return super().do_GET()
        if path == '/database':
            self.send_response(200)
            self.send_header('Content-Type', 'application/vnd.sqlite3')
            self.send_header('Content-Length', str(self.database.stat().st_size))
            self.send_header('Content-Disposition', 'attachment; filename="fixed-mesh-preview.sqlite3"')
            self.end_headers()
            with self.database.open('rb') as file:
                while chunk := file.read(65536):
                    self.wfile.write(chunk)
            return
        part = re.fullmatch(r'/adaptive-part/(fuji|yari)/(2|5|10|20|30|50|100|250|500|1000)/(\d{1,2})/(\d{1,2})', path)
        if part:
            with sqlite3.connect(self.database.resolve().as_uri()+'?mode=ro', uri=True) as db:
                row=db.execute('SELECT data FROM adaptive_parts WHERE region=? AND tolerance=? AND x=? AND y=?',(part[1],int(part[2]),int(part[3]),int(part[4]))).fetchone()
            if not row:return self.send_error(404)
            body=row[0]
            self.send_response(200)
            self.send_header('Content-Type','application/octet-stream')
            self.send_header('Content-Length',str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        hierarchy = re.fullmatch(r'/hierarchy/(fuji|yari)/([1-7])/(\d{1,2})/(\d{1,2})/(mesh|normal|normal_lod)', path)
        if hierarchy:
            with sqlite3.connect(self.database.resolve().as_uri()+'?mode=ro', uri=True) as db:
                row = db.execute('SELECT data FROM hierarchy_blobs WHERE region=? AND level=? AND x=? AND y=? AND kind=?', (hierarchy[1], int(hierarchy[2]), int(hierarchy[3]), int(hierarchy[4]), hierarchy[5])).fetchone()
            if not row:
                return self.send_error(404)
            body=self.normal_body(row[0]) if hierarchy[5].startswith('normal') else row[0]
            self.send_response(200)
            self.send_header('Content-Type', 'image/png' if hierarchy[5].startswith('normal') else 'application/octet-stream')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        match = re.fullmatch(r'/blob/(fuji|yari)/(height|mesh|normal|curve|slope|adaptive(?:_min(?:32|64|128|256|512|1024))?|vertex_normal|vertex_normal_slope|vertex_normal_adaptive(?:_min(?:32|64|128|256|512|1024))?)/(\d+)', path)
        if path == '/manifest' or match:
            with sqlite3.connect(self.database.resolve().as_uri()+'?mode=ro', uri=True) as db:
                if match and match[2] in ('curve','slope'):
                    row=db.execute('SELECT data FROM '+('curve_blobs' if match[2]=='curve' else 'slope_blobs')+' WHERE region=? AND variant=?',(match[1],int(match[3]))).fetchone()
                else:row = (db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone() if not match else
                       db.execute('SELECT data FROM blobs WHERE region=? AND kind=? AND variant=?',
                                  (match[1], match[2], int(match[3]))).fetchone())
            if not row:
                return self.send_error(404)
            body = row[0].encode('utf-8') if not match else row[0]
            if match and match[2]=='normal':body=self.normal_body(body)
            mime = 'application/json; charset=utf-8' if not match else 'image/png' if match[2] == 'normal' else 'application/octet-stream'
            self.send_response(200)
            self.send_header('Content-Type', mime)
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if path not in ('/fixed_mesh.html', '/fixed_mesh.js', '/height-mesh.js', '/pn-terrain.js', '/curve-terrain.js'):
            return self.send_error(404)
        super().do_GET()


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--port', type=int, default=61442)
    parser.add_argument('--database', type=Path, default=Handler.database)
    args = parser.parse_args()
    Handler.database = args.database
    ThreadingHTTPServer(('127.0.0.1', args.port), partial(Handler, directory=str(Path(__file__).parent))).serve_forever()
