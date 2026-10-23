"""親頂点を保持する地域TINの保存形式と復元時間を比較する。"""
import argparse
import json
import sqlite3
import struct
import time
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUTPUT = ROOT / 'build/hierarchical-lod-20261023'
REGIONS = {'fuji': (35.3606, 138.7274), 'yari': (36.3419, 137.6476), 'tsukuba': (36.2254, 140.1067)}


def prepare():
    import numpy as np
    from fixed_mesh_preview import source_grid
    OUTPUT.mkdir(exist_ok=True)
    with sqlite3.connect((ROOT/'build/terrain-lod.sqlite3').resolve().as_uri()+'?mode=ro', uri=True) as db:
        for region, (lat, lon) in REGIONS.items():
            grid, _ = source_grid(db, lat, lon, extent=16000, size=513)
            grid.astype('<f8').tofile(OUTPUT/(region+'.f64'))


def pack():
    report = {'extent': 16000, 'scope': '同一16 km範囲・共通DEM・最大誤差優先の連続挿入。全国の可変サイズ保存タイルへの適用は未検証。', 'regions': {}}
    database = OUTPUT/'comparison.sqlite3'
    with sqlite3.connect(database) as db:
        db.execute('CREATE TABLE IF NOT EXISTS blobs(region TEXT,method TEXT,level INTEGER,kind TEXT,data BLOB,PRIMARY KEY(region,method,level,kind))')
        for region in REGIONS:
            stages = json.loads((OUTPUT/(region+'.json')).read_text())
            previous_points, previous_faces = [], {}
            encoded = {method: [] for method in ['independent', 'shared', 'delta']}
            summary = []
            for level, stage in enumerate(stages):
                points = stage['Points']
                assert points[:len(previous_points)] == previous_points
                assert len(points) <= 65535
                vertices = b''.join(struct.pack('<3H', round(p[0]/31.25), round(p[1]/31.25), round(p[2]*4)) for p in points)
                added = vertices[len(previous_points)*6:]
                faces = dict(zip(stage['IDs'], map(tuple, stage['Faces'])))
                edges = {}
                area = 0
                xy = [(round(p[0]/31.25), round(p[1]/31.25)) for p in points]
                for f in faces.values():
                    a, b, c = (xy[i] for i in f)
                    cross = (b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])
                    assert cross > 0
                    area += cross
                    for j in range(3):
                        a, b, c = f[j], f[(j+1)%3], f[(j+2)%3]
                        assert (a, b) not in edges
                        if (b, a) in edges:
                            p = xy[edges[b, a]]
                            ax, ay = xy[a][0]-p[0], xy[a][1]-p[1]
                            bx, by = xy[b][0]-p[0], xy[b][1]-p[1]
                            cx, cy = xy[c][0]-p[0], xy[c][1]-p[1]
                            determinant = (ax*ax+ay*ay)*(bx*cy-by*cx)-(bx*bx+by*by)*(ax*cy-ay*cx)+(cx*cx+cy*cy)*(ax*by-ay*bx)
                            assert determinant <= 0, 'Delaunay条件違反'
                        edges[a, b] = c
                assert area == 2*512*512
                indices = b''.join(struct.pack('<3H', *f) for f in faces.values())
                removed = sorted(set(previous_faces)-set(faces))
                new = sorted(set(faces)-set(previous_faces))
                delta = struct.pack('<II', len(removed), len(new))+b''.join(struct.pack('<I', i) for i in removed)+b''.join(struct.pack('<I3H', i, *faces[i]) for i in new)
                row = {'error': stage['Error'], 'vertices': len(points), 'inherited': len(previous_points), 'triangles': len(faces), 'maxError': stage['MaxError'], 'minAngle': stage['MinAngle'], 'bytes': {}}
                for method, v, f in [('independent', vertices, indices), ('shared', added, indices), ('delta', added, delta)]:
                    pair = (zlib.compress(v, 6), zlib.compress(f, 6))
                    encoded[method].append(pair)
                    row['bytes'][method] = sum(map(len, pair))
                    for kind, body in zip(['vertices', 'triangles'], pair):
                        db.execute('INSERT OR REPLACE INTO blobs VALUES(?,?,?,?,?)', (region, method, level, kind, body))
                summary.append(row)
                previous_points, previous_faces = points, faces

            def restore(method):
                points = bytearray()
                faces = {}
                meshes = []
                for vb, ib in encoded[method]:
                    v, f = zlib.decompress(vb), zlib.decompress(ib)
                    if method == 'independent':
                        points = bytearray(v)
                    else:
                        points.extend(v)
                    if method == 'delta':
                        nr, na = struct.unpack_from('<II', f)
                        offset = 8
                        for _ in range(nr):
                            faces.pop(struct.unpack_from('<I', f, offset)[0]); offset += 4
                        for _ in range(na):
                            i, a, b, c = struct.unpack_from('<I3H', f, offset); offset += 10
                            assert i not in faces
                            faces[i] = (a, b, c)
                        assert offset == len(f)
                        f = b''.join(struct.pack('<3H', *faces[i]) for i in sorted(faces))
                    meshes.append((bytes(points), f))
                return meshes

            reference = restore('independent')
            timings = {}
            for method in encoded:
                assert restore(method) == reference, method
                start = time.perf_counter()
                for _ in range(30):
                    restore(method)
                timings[method] = (time.perf_counter()-start)*1000/30
            report['regions'][region] = {'stages': summary, 'totalBytes': {m: sum(sum(map(len, pair)) for pair in pairs) for m, pairs in encoded.items()}, 'restoreAllMsPython': timings}
        db.execute('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT)')
        db.execute('INSERT OR REPLACE INTO metadata VALUES(?,?)', ('report', json.dumps(report, ensure_ascii=False)))
    (OUTPUT/'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
    (OUTPUT/'index.html').write_text((Path(__file__).with_name('hierarchical_lod_compare.html')).read_text(encoding='utf-8'), encoding='utf-8')
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('action', choices=['prepare', 'pack'])
    args = parser.parse_args()
    prepare() if args.action == 'prepare' else pack()
