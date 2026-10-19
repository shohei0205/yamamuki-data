"""RustのPNG Adaptive TIN出力を既存の比較SQLiteへ追加する。"""
import argparse
import hashlib
import json
from pathlib import Path
import sqlite3
import struct
import subprocess
import zlib

import numpy as np
from PIL import Image
from scipy.ndimage import gaussian_filter, map_coordinates
from vertex_normals import encode_oct, decode_oct

ROOT = Path(__file__).resolve().parents[2]


def generate(database=ROOT/'build/fixed-mesh-preview.sqlite3', min_vertices=4, tolerances=(1000, 500, 250, 100, 50, 20, 10), max_vertices=200000):
    binary = ROOT/'build/terrain-rust-target/release/terrain-adaptive-tin.exe'
    with sqlite3.connect(database) as db:
        manifest = json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
        for region, meta in manifest['regions'].items():
            size = meta['gridSize']
            grid = np.frombuffer(zlib.decompress(db.execute('SELECT data FROM blobs WHERE region=? AND kind="height" AND variant=?', (region, size)).fetchone()[0]), '<f4').reshape(size, size)
            offset = min(0, float(np.floor(grid.min())))
            pixels = np.rint((grid-offset)*4)
            if not np.isfinite(pixels).all() or pixels.min()<0 or pixels.max()>65535:
                raise ValueError('地域標高を0.25 m単位のPNGへ保存できません')
            source = ROOT/f'build/adaptive-tin-{region}-height.png'
            Image.fromarray(pixels.astype(np.uint16)).save(source)
            records = []
            for tolerance in tolerances:
                base = next((m for m in meta.get('adaptiveMeshes', []) if m['tolerance']==tolerance), None)
                if min_vertices>4 and base and base['vertices']>=min_vertices:
                    records.append(dict(base, minimumVertices=min_vertices, blobKind='adaptive'))
                    continue
                kind = 'adaptive' if min_vertices==4 else f'adaptive_min{min_vertices}'
                output = ROOT/f'build/{kind}-{region}-{tolerance}.json'
                process = subprocess.run([str(binary), '--input', str(source), '--output', str(output), '--max-vertices', str(max_vertices), '--min-vertices', str(min_vertices), '--max-error', str(tolerance), '--scale', '0.25', '--offset', str(offset)], check=True, capture_output=True, text=True, encoding='utf-8')
                mesh = json.loads(output.read_text(encoding='utf-8'))
                points = np.asarray(mesh['vertices'], dtype=np.float64)
                pixel_xy = points[:, :2].copy()
                points[:, 0] = (points[:, 0]/(size-1)-.5)*meta['extent']
                points[:, 1] = (.5-points[:, 1]/(size-1))*meta['extent']
                faces = np.asarray(mesh['triangles'], dtype='<u4')[:, [0, 2, 1]]
                raw = struct.pack('<II', len(points), faces.size)+points.astype('<f4').tobytes()+faces.tobytes()
                body = zlib.compress(raw, 9)
                # 元の地域標高から、既存比較と同じ頂点密度依存の局所平均法線を作る。
                sigma = max(.5, (size-1)/(np.sqrt(len(points))*4))
                dy, dx = np.gradient(gaussian_filter(grid.astype(float), sigma, mode='nearest'), meta['extent']/(size-1))
                x, y = pixel_xy.T
                normals = np.column_stack([-map_coordinates(dx, [y, x], order=1, mode='nearest'), map_coordinates(dy, [y, x], order=1, mode='nearest'), np.ones(len(points))])
                normals /= np.linalg.norm(normals, axis=1, keepdims=True)
                octets = encode_oct(normals)
                angles = np.degrees(np.arccos(np.clip((normals*decode_oct(octets)).sum(axis=1), -1, 1)))
                packet = zlib.compress(struct.pack('<4sI', b'YVN1', len(points))+hashlib.sha256(raw).digest()+octets.tobytes(), 9)
                db.execute('INSERT OR REPLACE INTO blobs VALUES(?,?,?,?)', (region, kind, tolerance, body))
                db.execute('INSERT OR REPLACE INTO blobs VALUES(?,?,?,?)', (region, 'vertex_normal_'+kind, tolerance, packet))
                record = dict(minimumVertices=min_vertices,minimumVerticesReached=mesh.get('minimumVerticesReached',True),blobKind=kind,tolerance=tolerance, vertices=len(points), triangles=len(faces), bytes=len(body), vertexNormalBytes=len(packet), maxError=mesh['maxError'], rmse=None, toleranceReached=mesh['toleranceReached'], generationSeconds=json.loads(process.stdout)['seconds'], maxAngle=float(angles.max()), meanAngle=float(angles.mean()), sigmaMeters=sigma*meta['extent']/(size-1))
                records.append(record)
                print(region, json.dumps(record), flush=True)
            if min_vertices==4:
                merged={m['tolerance']:m for m in meta.get('adaptiveMeshes', [])}
                merged.update({m['tolerance']:m for m in records})
                meta['adaptiveMeshes'] = sorted(merged.values(), key=lambda m:m['tolerance'])
                for floor, existing in meta.get('adaptiveFloors', {}).items():
                    known={r['tolerance'] for r in existing}
                    existing.extend(dict(r, minimumVertices=int(floor), blobKind='adaptive') for r in records if r['tolerance'] not in known and r['vertices']>=int(floor))
                    existing.sort(key=lambda r:r['tolerance'])
            else:meta.setdefault('adaptiveFloors', {})[str(min_vertices)]=sorted(records, key=lambda m:m['tolerance'])
            meta['adaptiveSource'] = '0.25 m単位の地域PNGに対する誤差。表示用Float32丸め前。'
            db.execute('UPDATE metadata SET value=? WHERE key="manifest"', (json.dumps(manifest, ensure_ascii=False),))
            db.commit()


if __name__ == '__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--min-vertices',type=int,default=4)
    parser.add_argument('--tolerances',type=int,nargs='+',default=[1000,500,250,100,50,20,10])
    parser.add_argument('--max-vertices',type=int,default=200000)
    args=parser.parse_args()
    if args.min_vertices not in (4,32,64,128,256,512,1024):parser.error('最低頂点数は4・32・64・128・256・512・1024から指定してください')
    if any(t<=0 for t in args.tolerances):parser.error('誤差は正の整数で指定してください')
    if args.max_vertices<args.min_vertices:parser.error('最大頂点数は最低頂点数以上にしてください')
    generate(min_vertices=args.min_vertices,tolerances=sorted(set(args.tolerances),reverse=True),max_vertices=args.max_vertices)
