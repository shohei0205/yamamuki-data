"""単一 TIN と複数解像度の法線を、地域比較用 SQLite へ保存する。"""
import argparse
from io import BytesIO
import json
import math
from pathlib import Path
import sqlite3
import struct
import zlib

import numpy as np
from PIL import Image
from scipy.ndimage import map_coordinates
from scipy.spatial import Delaunay

from compact_dem import decode
from irregular_dem import simplify_irregular

ROOT = Path(__file__).resolve().parents[2]
WORLD = 40075016.68557849
REGIONS = {'fuji': (35.3606, 138.7274), 'yari': (36.3419, 137.6476)}


def source_grid(db, lat, lon, extent=24000, size=1025):
    """最高レベルの保存済み標高を接合し、地域全体の格子へ補間する。"""
    cx = (lon + 180) / 360 * WORLD
    cy = (1 - math.asinh(math.tan(math.radians(lat))) / math.pi) / 2 * WORLD
    scale = math.cos(math.radians(lat))
    pixel = WORLD / (2**14 * 256)
    left, top = (cx - extent / scale / 2) / pixel, (cy - extent / scale / 2) / pixel
    right, bottom = left + extent / scale / pixel, top + extent / scale / pixel
    west, north = math.floor((left - 1) / 256), math.floor((top - 1) / 256)
    east, south = math.floor((right + 1) / 256), math.floor((bottom + 1) / 256)
    data = np.full(((south - north + 1) * 256, (east - west + 1) * 256), np.nan, np.float32)
    for x, y, body in db.execute('SELECT x,y,data FROM tiles WHERE z=14 AND x BETWEEN ? AND ? AND y BETWEEN ? AND ?', (west, east, north, south)):
        data[(y-north)*256:(y-north+1)*256, (x-west)*256:(x-west+1)*256] = decode(body)
    xs, ys = np.linspace(left, right, size)-west*256-.5, np.linspace(top, bottom, size)-north*256-.5
    yy, xx = np.meshgrid(ys, xs, indexing='ij')
    grid = map_coordinates(data, [yy, xx], order=1, mode='nearest')
    if not np.isfinite(grid).all():
        raise ValueError('比較範囲に欠損があります。海岸を含まない範囲を選んでください')
    # 法線はメッシュ生成用格子より細かな、保存済み DEM の間隔から求める。
    dy, dx = np.gradient(data, pixel*scale)
    return grid, (data, dx, dy, left-west*256-.5, top-north*256-.5, right-left, bottom-top)


def normal_png(source, size):
    data, dx, dy, left, top, width, height = source
    # 詳細 DEM の法線を先に作り、方向を平均して縮小する。
    length = np.sqrt(dx*dx+dy*dy+1)
    full = np.stack([-dx/length, dy/length, 1/length], axis=-1)
    rows = []
    for axis in range(3):
        image = Image.fromarray(np.nan_to_num(full[:, :, axis], nan=0).astype(np.float32))
        image = image.transform((math.ceil(width), math.ceil(height)), Image.Transform.EXTENT,
                                (left, top, left+width, top+height), Image.Resampling.BILINEAR)
        image = image.resize((size, size), Image.Resampling.BOX)
        rows.append(np.asarray(image))
    normals = np.stack(rows, axis=-1)
    normals /= np.maximum(np.linalg.norm(normals, axis=-1, keepdims=True), 1e-10)
    out = BytesIO()
    Image.fromarray(np.rint((normals+1)*127.5).clip(0, 255).astype(np.uint8)).save(out, format='PNG', optimize=True)
    return out.getvalue()



def mesh_blob(grid, tolerance, extent=24000):
    size=len(grid)
    vertices, faces, error = simplify_irregular(grid, tolerance, max_iterations=2048, batch_size=2048)
    restored = Delaunay(vertices[:, :2])
    # 格子点全体の誤差を独立に再計算する。
    yy, xx = np.indices(grid.shape)
    points = np.column_stack([xx.ravel(), yy.ravel()])
    ids = restored.find_simplex(points)
    if np.any(ids < 0):
        raise ValueError('メッシュの範囲に穴があります')
    transforms = restored.transform[ids]
    weights = np.einsum('ijk,ik->ij', transforms[:, :2], points-transforms[:, 2])
    weights = np.column_stack([weights, 1-weights.sum(axis=1)])
    predicted = (vertices[restored.simplices[ids], 2]*weights).sum(axis=1)
    errors = predicted-grid.ravel()
    if np.max(np.abs(errors)) > tolerance+.01:
        raise ValueError('メッシュの誤差検証に失敗しました')
    vertices[:, 0] = vertices[:, 0]/(size-1)*extent-extent/2
    vertices[:, 1] = extent/2-vertices[:, 1]/(size-1)*extent
    raw = struct.pack('<II', len(vertices), len(faces)*3)+vertices.astype('<f4').tobytes()+faces.astype('<u4').tobytes()
    body = zlib.compress(raw, 9)
    return body, {'tolerance': tolerance, 'vertices': len(vertices), 'triangles': len(faces),
                  'bytes': len(body), 'maxError': float(np.max(np.abs(errors))),
                  'rmse': float(np.sqrt(np.mean(errors**2)))}


def append_fine(output, tolerances=(10, 5)):
    # 保存済みの同じ比較格子から追加し、既存の形状や法線は変更しない。
    with sqlite3.connect(output) as db:
        manifest=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
        for name, report in manifest['regions'].items():
            packed=db.execute('SELECT data FROM blobs WHERE region=? AND kind="height" AND variant=?',(name,report['gridSize'])).fetchone()[0]
            grid=np.frombuffer(zlib.decompress(packed),'<f4').reshape(report['gridSize'],report['gridSize'])
            for tolerance in tolerances:
                if any(m['tolerance']==tolerance for m in report['meshes']):continue
                body, stats=mesh_blob(grid,tolerance,report['extent'])
                db.execute('INSERT INTO blobs VALUES(?,?,?,?)',(name,'mesh',tolerance,body))
                report['meshes'].append(stats)
                report['meshes'].sort(key=lambda m:m['tolerance'])
                db.execute('UPDATE metadata SET value=? WHERE key="manifest"',(json.dumps(manifest,ensure_ascii=False),))
                db.commit()
                print(name,stats,flush=True)


def generate(source, output, tolerances=(5, 10, 20, 50, 100, 250, 500, 1000)):
    output.parent.mkdir(parents=True, exist_ok=True)
    if output.exists():
        raise FileExistsError('既存の比較 DB は上書きしません')
    with sqlite3.connect(source.resolve().as_uri()+'?mode=ro', uri=True) as src, sqlite3.connect(output) as db:
        db.execute('CREATE TABLE blobs(region TEXT,kind TEXT,variant INTEGER,data BLOB,PRIMARY KEY(region,kind,variant))')
        db.execute('CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT)')
        reports = {}
        for name, (lat, lon) in REGIONS.items():
            grid, normals = source_grid(src, lat, lon)
            size, extent = len(grid), 24000
            packed = zlib.compress(grid.astype('<f4').tobytes(), 9)
            db.execute('INSERT INTO blobs VALUES(?,?,?,?)', (name, 'height', size, packed))
            report = {'lat': lat, 'lon': lon, 'extent': extent, 'gridSize': size,
                      'heightBytes': len(packed), 'heightTriangles': 2*(size-1)**2,
                      'meshes': [], 'normals': []}
            for tolerance in tolerances:
                body, stats = mesh_blob(grid, tolerance, extent)
                db.execute('INSERT INTO blobs VALUES(?,?,?,?)', (name, 'mesh', tolerance, body))
                report['meshes'].append(stats)
                print(name, report['meshes'][-1], flush=True)
            for resolution in (32, 64, 128, 256, 512, 1024):
                body = normal_png(normals, resolution)
                db.execute('INSERT INTO blobs VALUES(?,?,?,?)', (name, 'normal', resolution, body))
                report['normals'].append({'resolution': resolution, 'bytes': len(body)})
            reports[name] = report
            db.commit()
        manifest = {'regions': reports, 'source': '国土地理院 DEM10B の保存済み L14 を加工',
                    'notes': '24 km 四方。形状誤差は1025四方の比較格子上で検証。詳細DEMの全点誤差や実測精度ではない。許容誤差の異なるメッシュは別候補で、表示中の形状LOD切替には使わない。'}
        db.execute('INSERT INTO metadata VALUES(?,?)', ('manifest', json.dumps(manifest, ensure_ascii=False)))
    print(output, output.stat().st_size, flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--source', type=Path, default=ROOT/'build/terrain-lod.sqlite3')
    parser.add_argument('--output', type=Path, default=ROOT/'build/fixed-mesh-preview.sqlite3')
    parser.add_argument('--append-fine', action='store_true')
    args = parser.parse_args()
    if args.append_fine:
        append_fine(args.output)
    else:
        generate(args.source, args.output)
