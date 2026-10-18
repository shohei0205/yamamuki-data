"""縮小画像を分割せず、各レベルの実際の隣接 2×2 標高タイルを用意する。"""
import json
import math
from pathlib import Path
import sqlite3

import numpy as np
from PIL import Image

from compact_dem import decode

ROOT = Path(__file__).resolve().parents[2]


def generate():
    with sqlite3.connect((ROOT/'build/terrain-lod.sqlite3').as_uri()+'?mode=ro', uri=True) as db:
        cache = {}
        def rect(z, x, y, w, h):
            out = np.full((h, w), np.nan, np.float32)
            for ty in range(y//256, (y+h-1)//256+1):
                for tx in range(x//256, (x+w-1)//256+1):
                    key = z, tx, ty
                    if key not in cache:
                        row = db.execute('SELECT data FROM tiles WHERE z=? AND x=? AND y=?', key).fetchone()
                        cache[key] = decode(row[0]) if row else None
                    tile = cache[key]
                    if tile is None:
                        continue
                    x0, y0 = max(x, tx*256), max(y, ty*256)
                    x1, y1 = min(x+w, (tx+1)*256), min(y+h, (ty+1)*256)
                    out[y0-y:y1-y, x0-x:x1-x] = tile[y0-ty*256:y1-ty*256, x0-tx*256:x1-tx*256]
            return out
        for region, (lat, lon) in {'yari': (36.3419, 137.6476), 'tsukuba': (36.2254, 140.1069)}.items():
            root = ROOT/f'build/terrain-tile-preview-{region}'
            levels = []
            for z in range(14, 3, -1):
                cx = int((lon+180)/360*2**z)
                cy = int((1-math.asinh(math.tan(math.radians(lat)))/math.pi)/2*2**z)
                west, north = cx-1, cy-1
                heights = rect(z, west*256, north*256, 512, 512)
                valid = np.isfinite(heights)
                offset = math.floor(float(heights[valid].min())) if valid.any() else 0
                q = np.rint(np.where(valid, heights-offset, 0)).clip(0, 65535).astype(np.uint16)
                folder = root/'levels'/str(z)
                (folder/'normals').mkdir(parents=True, exist_ok=True)
                Image.fromarray(np.stack([q&255, q>>8, valid*255], axis=-1).astype(np.uint8)).save(folder/'height.png')
                nz = min(14, z+1)
                factor = 2**(nz-z)
                nsize = 512*factor
                grid = rect(nz, west*256*factor-1, north*256*factor-1, nsize+2, nsize+2)
                center = grid[1:-1, 1:-1]
                center_lat = math.atan(math.sinh(math.pi*(1-2*(north+1)/2**z)))
                meters = 2*math.pi*6378137*math.cos(center_lat)/(256*2**z)
                dx = (np.where(np.isfinite(grid[1:-1, 2:]), grid[1:-1, 2:], center)-np.where(np.isfinite(grid[1:-1, :-2]), grid[1:-1, :-2], center))/(2*meters/factor)
                dy = (np.where(np.isfinite(grid[2:, 1:-1]), grid[2:, 1:-1], center)-np.where(np.isfinite(grid[:-2, 1:-1]), grid[:-2, 1:-1], center))/(2*meters/factor)
                length = np.sqrt(1+dx*dx+dy*dy)
                nx = np.where(np.isfinite(center), -dx/length, 0)
                ny = np.where(np.isfinite(center), dy/length, 0)
                variants = []
                for size in [1024, 512] if nsize == 1024 else [512]:
                    xy = np.stack([np.asarray(Image.fromarray(v).resize((size, size), Image.Resampling.BOX)) for v in [nx, ny]], axis=-1)
                    normal = np.full((size, size, 3), 255, np.uint8)
                    normal[..., :2] = np.rint((xy+1)*127.5).clip(0, 255).astype(np.uint8)
                    path = folder/'normals'/f'{size}.png'
                    Image.fromarray(normal).save(path)
                    variants.append(dict(size=size, bytes=path.stat().st_size, url=f'/levels/{z}/normals/{size}.png'))
                meta = dict(level=z, zoom=z, west=west, north=north, width=512, height=512, heightOffset=offset, heightScale=1,
                            meters=meters, heightBytes=(folder/'height.png').stat().st_size, tileColumns=2, tileRows=2,
                            normalSourceLevel=nz, normalVariants=variants,
                            source='保存済み全国 DEM の実際の隣接 2×2 タイル。法線は最大で 1 段詳細な DEM から作成。')
                (folder/'manifest.json').write_text(json.dumps(meta, ensure_ascii=False), encoding='utf-8')
                levels.append(meta)
            (root/'levels.json').write_text(json.dumps(dict(levels=levels, normalVariants=levels[0]['normalVariants']), ensure_ascii=False), encoding='utf-8')
            print(region, len(levels), 'levels')


if __name__ == '__main__':
    generate()
