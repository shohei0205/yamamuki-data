"""Rust階層メッシュのBLOBを既存の地域テストHTMLへ配信する。"""
import argparse
import json
import math
import sqlite3
import struct
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[2]
REGIONS = [('fuji', '富士山', 138.7274, 35.3606, 3776),
           ('yari', '槍ヶ岳', 137.6476, 36.3419, 3180),
           ('tsukuba', '筑波山', 140.1067, 36.2254, 877)]
HEADER = struct.Struct('<4s6i7I')


def project(lon, lat):
    p1, p2 = map(math.radians, (30, 46))
    n = math.log(math.cos(p1)/math.cos(p2))/math.log(math.tan(math.pi/4+p2/2)/math.tan(math.pi/4+p1/2))
    f = math.cos(p1)*math.tan(math.pi/4+p1/2)**n/n
    rho0 = 6378137*f/math.tan(math.pi/4+math.radians(36)/2)**n
    rho = 6378137*f/math.tan(math.pi/4+math.radians(lat)/2)**n
    t = n*math.radians(lon-138)
    return rho*math.sin(t), rho0-rho*math.cos(t)


def connect(path):
    return sqlite3.connect(path.resolve().as_uri()+'?mode=ro', uri=True)


def manifest(path):
    with connect(path) as db:
        config = json.loads(db.execute("SELECT value FROM metadata WHERE key='hierarchy_config'").fetchone()[0])
        levels = [dict(level=q-3, q=q, widthMeters=2000*2**(q-4), maxErrorMeters=12.5*2**(q-4))
                  for q in range(config['minimumQ'], config['maximumQ']+1)]
        rows = db.execute('SELECT q,x,y,vertices,triangles,inherited,length(parent_refs)+length(new_vertices)+length(indices)+length(border_indices) FROM tiles ORDER BY q,y,x').fetchall()
        for spec in levels:
            if not db.execute("SELECT 1 FROM metadata WHERE key=?", (f"q{spec['q']}-complete",)).fetchone():
                raise ValueError('表示対象の生成が未完了です')
    if not all(0 <= spec['level'] <= 7 for spec in levels):
        raise ValueError('この地域HTMLで扱えるのはQ4～Q10です')
    regions = []
    max_q = config['maximumQ']
    root_width = 2000*2**(max_q-4)
    for region_id, name, lon, lat, height in REGIONS:
        cx, cy = project(lon, lat)
        roots = {(r[1], r[2]) for r in rows if r[0] == max_q and r[1]*root_width < cx+16000 and (r[1]+1)*root_width > cx-16000 and r[2]*root_width < cy+16000 and (r[2]+1)*root_width > cy-16000}
        reports = []
        for spec in levels:
            factor = 2**(max_q-spec['q'])
            selected = [r for r in rows if r[0] == spec['q'] and (r[1]//factor, r[2]//factor) in roots]
            if not selected:
                raise ValueError(f'{name} Q{spec["q"]}のタイルがありません')
            work = path.with_suffix('.work')/f'q{spec["q"]}-basic.sqlite3'
            with connect(work) as db:
                base_vertices = sum(db.execute('SELECT vertices FROM basic WHERE q=? AND x=? AND y=?', r[:3]).fetchone()[0] for r in selected)
            reports.append(dict(level=spec['level'], q=spec['q'], widthMeters=spec['widthMeters'],
                                tileCount=len(selected), keys=[[r[0]-3, r[1], r[2]] for r in selected],
                                vertices=sum(r[3] for r in selected), triangles=sum(r[4] for r in selected),
                                inherited=sum(r[5] for r in selected), bytes=sum(r[6] for r in selected),
                                baseVertices=base_vertices, addedVertices=sum(r[3] for r in selected)-base_vertices))
        regions.append(dict(id=region_id, name=name, lon=lon, lat=lat, height=height, levels=reports))
    return dict(hierarchyPreview=True, regionalLevels=True, priorityComparison=True, priorityMode='rust-hierarchy',
                coastlineInMesh=False, demSlopeHints=False, levels=levels, reliefRegions=regions,
                tiles=[[q-3, x, y, size, 0, 0, 0] for q,x,y,nv,nf,inherit,size in rows],
                triangleEncoding='TFC1', outerCompression='none', boundaryConnectionComplete=False)


class Handler(SimpleHTTPRequestHandler):
    database = None
    data_manifest = None

    def end_headers(self):
        self.send_header('Cache-Control', 'no-store')
        super().end_headers()

    def send_bytes(self, body, content_type):
        self.send_response(200)
        self.send_header('Content-Type', content_type)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        route = urlsplit(self.path).path
        if route == '/manifest':
            self.send_bytes(json.dumps(self.data_manifest, ensure_ascii=False).encode(), 'application/json; charset=utf-8')
            return
        if route.startswith('/hierarchy/'):
            try:
                q, x, y = map(int, route.removeprefix('/hierarchy/').split('/'))
            except ValueError:
                return self.send_error(400)
            with connect(self.database) as db:
                row = db.execute('SELECT parent_q,parent_x,parent_y,vertices,triangles,inherited,parent_refs,new_vertices,indices,border_indices FROM tiles WHERE q=? AND x=? AND y=?', (q,x,y)).fetchone()
            if not row:
                return self.send_error(404)
            pq, px, py, nv, nf, inherited, refs, vertices, triangles, borders = row
            body = HEADER.pack(b'YHD1', q,x,y,pq,px,py,nv,nf,inherited,len(refs),len(vertices),len(triangles),len(borders))+refs+vertices+triangles+borders
            self.send_bytes(body, 'application/octet-stream')
            return
        if route == '/triangle_frontier_decoder.wasm':
            self.send_bytes((ROOT/'build/hierarchical-lod-20261023/triangle_frontier_decoder.wasm').read_bytes(), 'application/wasm')
            return
        allowed = {'/national_mesh.html','/national_mesh.js','/national_mesh_worker.js','/hierarchy-tile.js',
                   '/triangle_frontier_wasm.mjs','/loop-gpu.js','/landcover-astc.js','/surface-methods.js',
                   '/hct-terrain.js','/pn-terrain.js','/stitch-terrain.js','/adaptive-curve.js'}
        if route in ('/', '/national_mesh.html'):
            html = (ROOT/'tools/terrain/national_mesh.html').read_text(encoding='utf-8')
            html = html.replace('<title>全国 Adaptive TIN</title>', '<title>Rust・地域メッシュ比較</title>')
            html = html.replace('全国 Adaptive TIN／独立LOD＋傾きヒント', 'Rust／親頂点共有LOD＋TFC1')
            self.send_bytes(html.encode(), 'text/html; charset=utf-8')
            return
        if route not in allowed:
            return self.send_error(404)
        return super().do_GET()


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--input', type=Path, default=ROOT/'build/hierarchy-rust-centered-preview-20261023.sqlite3')
    parser.add_argument('--port', type=int, default=61455)
    args = parser.parse_args()
    Handler.database = args.input.resolve()
    Handler.data_manifest = manifest(Handler.database)
    ThreadingHTTPServer(('127.0.0.1', args.port), partial(Handler, directory=str(ROOT/'tools/terrain'))).serve_forever()
