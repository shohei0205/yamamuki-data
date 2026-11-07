"""Serve the Fuji vector LOD prototype without changing existing terrain servers."""
import argparse
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
import json
from os import fstat
import gzip,sqlite3,zlib
from contextlib import closing
from functools import lru_cache
from urllib.parse import parse_qs,urlparse
import shapely
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
FILTER_ROOT=ROOT/'build/landcover-raster-fuji-v4'
FILTERS={'none','majority','regions','majority-clean'}
FIT_ROOT=ROOT/'build/landcover-bezier-fit-fuji-v5'
CLEAN_ROOT=ROOT/'build/landcover-native-water-holes-fuji-v1'
CLEAN_FIT_ROOT=ROOT/'build/landcover-native-water-holes-bezier-fuji-v1'
OVERLAY_FIT_ROOT=ROOT/'build/landcover-water-breaks-fuji-inlet-v9'
OVERLAY_PRIOR_ROOT=ROOT/'build/landcover-water-breaks-fuji-inlet-v8'
WIDE_RASTER_ROOT=ROOT/'build/landcover-fuji-256-raster'
WIDE_FILTER_ROOT=ROOT/'build/landcover-fuji-256-compat'
WIDE_NONE_PREVIEW=ROOT/'build/landcover-fuji-256-seams-smoke/none'
WIDE_FIT_ROOT=ROOT/'build/landcover-fuji-256-fit'
def verified_fine_overlays():
    roots={level:ROOT/f'build/landcover-fuji-256-l{level}-pixel-compact-v2/majority-clean'
           for level in (3,)}
    revised=ROOT/'build/landcover-fuji-256-l3-common-linear-final-v3/majority-clean'
    if (revised/'manifest.json').is_file():
        roots[3]=revised
    return {level:root for level,root in roots.items() if level==3 and root==revised or (root/'pixel-check.ok').is_file()}
WIDE_OVERLAY_ROOT=ROOT/'build/landcover-fuji-256-l4-common-linear-final-v3'
WIDE_PRIOR_ROOT=ROOT/'build/landcover-fuji-256-thin-spit-v2-l4'
WIDE_L5_RASTER=ROOT/'build/landcover-fuji-256-l5-raster/majority-clean'
WIDE_L5_OVERLAY=ROOT/'build/landcover-fuji-256-l5-common-linear-final-v4/majority-clean'
WIDE_L5_PRIOR=ROOT/'build/landcover-fuji-256-l5-tolerance2000/majority-clean'
WIDE_L6=ROOT/'build/landcover-fuji-256-l6-water-guard-v2/majority-clean'
PRIOR_CLEAN_FIT_ROOT=ROOT/'build/landcover-native-water-balanced-bezier-fuji-v1'
def dataset(filter_id,fitted=False,previous=False):
    if filter_id=='majority-clean':return (PRIOR_CLEAN_FIT_ROOT if previous else CLEAN_FIT_ROOT if fitted else CLEAN_ROOT)/filter_id
    return (FIT_ROOT if fitted else FILTER_ROOT)/filter_id
def triangulate_indices(poly):
    rings=[list(poly.exterior.coords)[:-1],*[list(r.coords)[:-1]for r in poly.interiors]]
    points=[p for ring in rings for p in ring]
    lookup={p:i for i,p in enumerate(points)}
    triangles=shapely.constrained_delaunay_triangles(poly)
    indices=[]
    for tri in triangles.geoms:
        indices.extend(lookup[p]for p in list(tri.exterior.coords)[:3])
    return indices

@lru_cache(maxsize=128)
def robust_polygon(data,level,code,index,strength=0.,steps=4):
    if strength:
        from landcover_curves import curved
        _,groups,_=curved(data,level,strength,steps);g=groups[code]
    else:
        with closing(sqlite3.connect((Path(data)/'landcover-vectors.sqlite3').as_uri()+'?mode=ro',uri=True))as db:
            row=db.execute("SELECT geometry FROM features WHERE level=? AND class=?",(level,code)).fetchone()
        if row is None:raise ValueError("Unknown layer")
        g=shapely.from_wkb(zlib.decompress(row[0]))
    polys=[g]if g.geom_type=="Polygon"else list(g.geoms)
    if not 0<=index<len(polys):raise ValueError("Unknown polygon")
    indices=triangulate_indices(polys[index])
    return gzip.compress(json.dumps({"indices":indices}).encode(),mtime=0)

def national_progress():
    directory = ROOT/'build/landcover-national-regions-v1'
    source = ROOT/'build/terrain-worldcover-source'
    def read_json(path, default):
        try:
            return json.loads(path.read_bytes())
        except (OSError, ValueError):
            return default
    plan = read_json(source/'japan-work-areas.json', [])
    gaps = {(r['x'], r['y']): r['esa'] for r in read_json(directory/'source-gaps.json', [])}
    audit = read_json(directory/'seam-audit.json', {})
    delivery = read_json(directory/'delivery.json', None)
    downloads = read_json(source/'japan-download-manifest.json', {})
    def ready(area, prefix):
        return any((path/'manifest.json').is_file() for path in area.glob(prefix+'*')
                   if path.name == prefix or path.name.startswith(prefix+'-retry-'))
    areas = []
    for tile in plan:
        area = directory/f"{tile['x']}-{tile['y']}"
        stages = []
        for level in range(3, 7):
            stages.append('仕上げ済み' if ready(area, f'l{level}') else
                          '簡略化済み' if ready(area, f'stage-l{level}') else
                          '元ベクター化済み' if ready(area, f'raw-l{min(level, 5)}')
                          or any((path/'manifest.json').is_file() and
                                 (path/'majority-clean'/f'level-{min(level, 5)}.geojson.gz').is_file()
                                 for path in area.glob('raw-shared*')) else '未完了')
        areas.append({**tile, 'done': (area/'finished.json').is_file(),
                      'started': area.is_dir(), 'stages': stages, 'sourceGap': gaps.get((tile['x'], tile['y']))})
    logs = {}
    for name in ('pilot.log', 'pilot.err.log', 'national.log', 'national.err.log',
                 'national-worker-0.log', 'national-worker-0.err.log',
                 'national-worker-1.log', 'national-worker-1.err.log'):
        try:
            with (directory/name).open('rb') as file:
                size = file.seek(0, 2)
                file.seek(max(0, size - 12000))
                data = file.read()
                modified = fstat(file.fileno()).st_mtime
            try:
                contents = data.decode('utf-8')
            except UnicodeDecodeError:
                contents = data.decode('cp932', errors='replace')
            logs[name] = {'tail': '\n'.join(contents.splitlines()[-12:]), 'updated': modified}
        except OSError:
            continue
    return {'areas': areas, 'sources': len(downloads.get('files', [])),
            'absent': len(downloads.get('absentFromCatalog', [])),
            'vrt': (source/'japan-2021-v200.vrt').is_file(), 'logs': logs, 'delivery': delivery,
            'seams': [{k: v for k, v in row.items() if k != 'edges'} for row in audit.get('levels', [])]}


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        url=self.path.split('?',1)[0]
        if url in ('/national-progress', '/national-progress.json'):
            if url.endswith('.json'):
                body = json.dumps(national_progress(), ensure_ascii=False).encode()
                mime = 'application/json; charset=utf-8'
            else:
                body = (ROOT/'tools/terrain/landcover_national_progress.html').read_bytes()
                mime = 'text/html; charset=utf-8'
            self.send_response(200)
            self.send_header('Content-Type', mime)
            self.send_header('Content-Length', str(len(body)))
            self.send_header('Cache-Control', 'no-store')
            self.end_headers()
            self.wfile.write(body)
            return
        if url == '/wide/xyz' or url.startswith('/xyz-data/'):
            directory = ROOT/'build/landcover-fuji-256-xyz-v3'
            if url == '/wide/xyz':
                body = (ROOT/'tools/terrain/landcover_xyz_fuji.html').read_bytes()
                mime, compressed = 'text/html; charset=utf-8', False
            elif url == '/xyz-data/manifest.json':
                if not (directory/'manifest.json').is_file():
                    self.send_error(404)
                    return
                body = (directory/'manifest.json').read_bytes()
                mime, compressed = 'application/json', False
            else:
                key = url.removeprefix('/xyz-data/').split('/')
                if len(key) != 3 or not all(len(v) <= 6 and v.isascii() and v.isdecimal() for v in key):
                    self.send_error(404)
                    return
                db_path = directory/'landcover-xyz.sqlite3'
                if not db_path.is_file():
                    self.send_error(404)
                    return
                with closing(sqlite3.connect(db_path.as_uri()+'?mode=ro', uri=True)) as db:
                    row = db.execute('SELECT payload FROM tiles WHERE z=? AND x=? AND y=?', tuple(map(int, key))).fetchone()
                if row is None or row[0] is None:
                    self.send_error(404)
                    return
                body, mime, compressed = row[0], 'application/octet-stream', True
            self.send_response(200)
            self.send_header('Content-Type', mime)
            self.send_header('Content-Length', str(len(body)))
            self.send_header('Cache-Control', 'no-cache')
            if compressed:
                self.send_header('Content-Encoding', 'gzip')
            self.end_headers()
            self.wfile.write(body)
            return
        wide=url=='/wide' or url.startswith('/wide/')
        if wide:
            url=url[5:] or '/'
        overlay_root=WIDE_OVERLAY_ROOT if wide else OVERLAY_FIT_ROOT
        prior_root=WIDE_PRIOR_ROOT if wide else OVERLAY_PRIOR_ROOT
        fine_overlays=verified_fine_overlays() if wide else {}
        def select_dataset(filter_id,fitted=False,previous=False):
            if wide:
                if filter_id=='majority-clean':
                    return (WIDE_FIT_ROOT if fitted else WIDE_RASTER_ROOT)/filter_id
                separate=ROOT/f'build/landcover-fuji-256-compat-{filter_id}'
                if (separate/filter_id/'manifest.json').is_file():
                    return (ROOT/f'build/landcover-fuji-256-compat-fit-{filter_id}' if fitted else separate)/filter_id
                if filter_id=='none' and not fitted and not (WIDE_FILTER_ROOT/'none/manifest.json').is_file():
                    return WIDE_NONE_PREVIEW
                return (ROOT/f'build/landcover-fuji-256-fit-{filter_id}' if fitted else WIDE_FILTER_ROOT)/filter_id
            return dataset(filter_id,fitted,previous)
        data_root=select_dataset('majority-clean') if wide else self.server.data
        if wide and url in ('/capabilities.json','/filters.json'):
            filters=sorted(f for f in FILTERS if (select_dataset(f)/'manifest.json').is_file())
            if url=='/capabilities.json':
                summary={'filters':filters,
                         'fitted':[f for f in filters if (select_dataset(f,True)/'manifest.json').is_file()],
                         'overlay':(overlay_root/'majority-clean/manifest.json').is_file(),
                         'fineOverlayLevels':list(fine_overlays),
                         'prior':(prior_root/'majority-clean/manifest.json').is_file(),
                         'relief':(WIDE_RASTER_ROOT/'relief.png').is_file(),
                         'nonePreview':select_dataset('none')==WIDE_NONE_PREVIEW}
            else:
                variants=[]
                for filter_id in filters:
                    row=json.loads((select_dataset(filter_id)/'manifest.json').read_bytes())
                    variants.append({**row['filter'],'levels':[v for v in row['levels'] if v['level']>=3],'sqliteBytes':row['sqliteBytes']})
                summary={'variants':variants}
            body=json.dumps(summary).encode();self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(body)));self.send_header('Cache-Control','no-cache');self.end_headers();self.wfile.write(body);return
        if url=='/filters.json':
            path=FILTER_ROOT/'manifest.json'
            if not path.is_file():self.send_error(404);return
            summary=json.loads(path.read_bytes())
            extra=CLEAN_ROOT/'manifest.json'
            if extra.is_file():summary['variants'].extend(json.loads(extra.read_bytes())['variants'])
            body=json.dumps(summary).encode();self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(body)));self.send_header('Cache-Control','no-cache');self.end_headers();self.wfile.write(body);return
        if url=='/curves':
            try:
                from landcover_curves import curved
                q=parse_qs(urlparse(self.path).query);level=int(q['level'][0]);strength=float(q.get('strength',['0.75'])[0]);steps=int(q.get('steps',['4'])[0])
                if level not in (range(3,7) if wide else range(5))or strength not in [.35,.75,1.]or steps not in [4,8]:raise ValueError("Invalid curve")
                body,_,_=curved(str({5:WIDE_L5_RASTER,6:WIDE_L6}.get(level,data_root) if wide else data_root),level,strength,steps)
                self.send_response(200);self.send_header('Content-Type','application/geo+json');self.send_header('Content-Encoding','gzip');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
            except Exception as e:self.send_error(400,str(e));return
        if url=='/triangulate':
            try:
                q=parse_qs(urlparse(self.path).query)
                level,code,index=(int(q[k][0])for k in ['level','class','polygon'])
                if level not in (range(3,7) if wide else range(5))or code not in range(5)or index<0:raise ValueError("Invalid polygon")
                strength=float(q.get('strength',['0'])[0]);steps=int(q.get('steps',['4'])[0])
                if strength not in [0.,.35,.75,1.]or steps not in [4,8]:raise ValueError('Invalid curve')
                filter_id=q.get('filter',['legacy'])[0]
                if filter_id!='legacy' and filter_id not in FILTERS:raise ValueError('Invalid filter')
                if filter_id!='legacy' and strength:raise ValueError('Filtered curves are not generated')
                fit_value=q.get('fit',['0'])[0]
                overlay_roots={'overlay':overlay_root,'overlay-prior':prior_root}
                fitted=fit_value in {'1','old'} or fit_value in overlay_roots
                if fit_value in overlay_roots and (filter_id!='majority-clean' or (level!=4 and level not in fine_overlays and not (wide and level in (5,6)))):raise ValueError('Unknown overlay data')
                if fitted and filter_id not in {'majority','regions','majority-clean'}:raise ValueError('Unknown fitted data')
                data=data_root if filter_id=='legacy' else overlay_roots[fit_value]/filter_id if fit_value in overlay_roots else select_dataset(filter_id,fitted,fit_value=='old')
                if level in fine_overlays and fit_value in overlay_roots:
                    data=fine_overlays[level] if fit_value=='overlay' else WIDE_FIT_ROOT/'majority-clean'
                if wide and level==6:
                    if filter_id!='majority-clean':raise ValueError('Unknown L6 data')
                    data=WIDE_L6
                if wide and level==5:
                    if (filter_id,fit_value) not in {('majority-clean','0'),('majority-clean','overlay'),('majority-clean','overlay-prior'),('legacy','0')}:raise ValueError('Unknown L5 data')
                    data={'overlay':WIDE_L5_OVERLAY,'overlay-prior':WIDE_L5_PRIOR}.get(fit_value,WIDE_L5_RASTER)
                body=robust_polygon(str(data),level,code,index,strength,steps)
                self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Encoding','gzip');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body);return
            except Exception as e:self.send_error(400,str(e));return
        if url=='/':path=ROOT/'tools/terrain/landcover_vector_fuji.html';mime='text/html; charset=utf-8';compressed=False
        elif wide and url=='/relief.png':path=WIDE_RASTER_ROOT/'relief.png';mime='image/png';compressed=False
        elif url=='/landcover_bezier_decode.js':path=ROOT/'tools/terrain/landcover_bezier_decode.js';mime='text/javascript';compressed=False
        elif url=='/vendor/earcut-3.2.4.min.js':path=ROOT/'tools/terrain/vendor/earcut-3.2.4.min.js';mime='text/javascript';compressed=False
        elif url.startswith(('/fitted-prior/','/fitted-overlay/','/fitted-overlay-prior/')):
            parts=url.split('/')
            if len(parts)!=4 or parts[2]!='majority-clean' or not parts[3] or '\\' in parts[3] or parts[3].startswith('.'):self.send_error(404);return
            root={'fitted-overlay':overlay_root,'fitted-overlay-prior':prior_root}.get(parts[1]);path=(root/parts[2] if root else select_dataset(parts[2],True,True))/parts[3];compressed=path.name.endswith('.gz')
            mime='application/octet-stream'if path.name.endswith('.curves.gz')else'application/json'
        elif url.startswith('/fitted/'):
            parts=url.split('/')
            if len(parts)!=4 or parts[2] not in {'majority','regions','majority-clean'} or not parts[3] or '\\' in parts[3] or parts[3].startswith('.'):self.send_error(404);return
            path=select_dataset(parts[2],True)/parts[3];compressed=path.name.endswith('.gz')
            mime='application/geo+json'if path.name.endswith('.geojson.gz')else'application/octet-stream'if path.name.endswith('.curves.gz')else'application/json'if path.name.endswith('.json')else'image/png'
        elif url.startswith('/filtered/'):
            parts=url.split('/')
            if len(parts)!=4 or parts[2] not in FILTERS or not parts[3] or '\\' in parts[3] or parts[3].startswith('.'):self.send_error(404);return
            path=select_dataset(parts[2])/parts[3];compressed=path.name.endswith('.geojson.gz')
            mime='application/geo+json'if compressed else'application/json'if path.name.endswith('.json')else'image/png'
        elif url.startswith('/data/'):
            name=url[6:]
            if '/'in name or '\\'in name or name.startswith('.'):self.send_error(404);return
            path=data_root/name;compressed=name.endswith('.geojson.gz')
            mime='application/geo+json'if compressed else'application/json'if name.endswith('.json')else'image/png'
        else:self.send_error(404);return
        extra=None
        if wide and (WIDE_L5_OVERLAY/'manifest.json').is_file():
            if path.parent==WIDE_RASTER_ROOT/'majority-clean':
                extra=WIDE_L5_RASTER
            elif path.parent==WIDE_OVERLAY_ROOT/'majority-clean':
                extra=WIDE_L5_OVERLAY
            elif path.parent==WIDE_PRIOR_ROOT/'majority-clean':
                extra=WIDE_L5_PRIOR
        coarse=wide and path.parent in (WIDE_RASTER_ROOT/'majority-clean',WIDE_OVERLAY_ROOT/'majority-clean') and (WIDE_L6/'manifest.json').is_file()
        if coarse and path.name in ('level-6.geojson.gz','level-6.curves.gz','reference-6.png'):
            path=WIDE_L6/path.name
        if extra and path.name in ('level-5.geojson.gz','level-5.curves.gz','reference-5.png'):
            path=extra/path.name
        fine={}
        if fine_overlays:
            if path.parent==WIDE_OVERLAY_ROOT/'majority-clean':
                fine=fine_overlays
            elif path.parent==WIDE_PRIOR_ROOT/'majority-clean':
                fine=dict.fromkeys(fine_overlays,WIDE_FIT_ROOT/'majority-clean')
        for level,root in fine.items():
            if path.name.startswith((f'level-{level}.', f'reference-{level}.')):
                path=root/path.name
                break
        if not path.is_file():self.send_error(404);return
        if wide and path.name.startswith(tuple(prefix for level in range(3)
                                               for prefix in (f'level-{level}.',f'reference-{level}.'))):
            self.send_error(404);return
        if (wide or extra or fine or coarse) and path.name=='manifest.json':
            manifest=json.loads(path.read_bytes())
            for level,root in fine.items():
                rows=json.loads((root/'manifest.json').read_bytes())['levels']
                manifest['levels'].extend(row for row in rows if row['level']==level)
            if extra:
                manifest['levels'].extend(json.loads((extra/'manifest.json').read_bytes())['levels'])
            if coarse:
                manifest['levels'].extend(json.loads((WIDE_L6/'manifest.json').read_bytes())['levels'])
            if wide:
                manifest['levels']=[row for row in manifest['levels'] if row['level']>=3]
            manifest['levels'].sort(key=lambda row:row['level'])
            body=json.dumps(manifest).encode()
        else:
            body=path.read_bytes()
        self.send_response(200);self.send_header('Content-Type',mime)
        if compressed:self.send_header('Content-Encoding','gzip')
        self.send_header('Content-Length',str(len(body)));self.send_header('Cache-Control','no-cache'if url=='/'or path.name=='manifest.json'else'public, max-age=3600');self.end_headers();self.wfile.write(body)
if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--port',type=int,default=61495);p.add_argument('--data',type=Path,default=ROOT/'build/landcover-vector-fuji-v2');a=p.parse_args()
    server=ThreadingHTTPServer(('127.0.0.1',a.port),Handler);server.data=a.data.resolve();print('http://127.0.0.1:'+str(a.port)+'/',flush=True);server.serve_forever()
