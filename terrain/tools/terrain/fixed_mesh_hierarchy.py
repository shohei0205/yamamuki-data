"""一辺が段階ごとに2倍になる広域メッシュをSQLiteへ事前保存する。"""
from io import BytesIO
import json

import numpy as np
from PIL import Image
from pathlib import Path
import sqlite3

from fixed_mesh_groups import atlas_png, group_mesh

ROOT = Path(__file__).resolve().parents[2]


def reduced_normal(body, resolution):
    # 法線を平均してから長さを1へ戻す。
    image = Image.open(BytesIO(body)).convert('RGB').resize((resolution, resolution), Image.Resampling.BOX)
    vectors = np.asarray(image, dtype=np.float32)/127.5-1
    vectors /= np.maximum(np.linalg.norm(vectors, axis=2, keepdims=True), 1e-8)
    output = BytesIO()
    Image.fromarray(np.rint(np.clip(vectors*.5+.5, 0, 1)*255).astype('uint8')).save(output, format='PNG')
    return output.getvalue()


def generate_normals(database=ROOT/'build/fixed-mesh-preview.sqlite3'):
    with sqlite3.connect(database) as db:
        manifest = json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
        resolutions = {level: 256//2**level for level in range(1, 8)}
        for region in manifest['regions']:
            source = db.execute('SELECT data FROM blobs WHERE region=? AND kind="normal" AND variant=256', (region,)).fetchone()[0]
            for level, resolution in resolutions.items():
                width = 2**level
                normal = reduced_normal(source, resolution)
                atlases = {}
                for y in range((100+width-1)//width):
                    for x in range((100+width-1)//width):
                        key = min(width, 100-x*width), min(width, 100-y*width)
                        if key not in atlases:
                            atlases[key] = atlas_png(normal, *key)
                        db.execute('INSERT OR REPLACE INTO hierarchy_blobs VALUES(?,?,?,?,?,?)',
                                   (region, level, x, y, 'normal_lod', atlases[key]))
                db.commit()
                print(region, '法線LOD', level, resolution, flush=True)
        manifest['hierarchy']['normalResolutions'] = resolutions
        db.execute('UPDATE metadata SET value=? WHERE key="manifest"', (json.dumps(manifest, ensure_ascii=False),))


def generate(database=ROOT/'build/fixed-mesh-preview.sqlite3'):
    with sqlite3.connect(database) as db:
        db.execute('CREATE TABLE IF NOT EXISTS hierarchy_blobs(region TEXT,level INTEGER,x INTEGER,y INTEGER,kind TEXT,data BLOB,PRIMARY KEY(region,level,x,y,kind))')
        manifest = json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
        for region in manifest['regions']:
            mesh = db.execute('SELECT data FROM blobs WHERE region=? AND kind="mesh" AND variant=1000', (region,)).fetchone()[0]
            normal = db.execute('SELECT data FROM blobs WHERE region=? AND kind="normal" AND variant=32', (region,)).fetchone()[0]
            atlases = {}
            for level in range(1, 8):
                width = 2**level
                for y in range((100+width-1)//width):
                    for x in range((100+width-1)//width):
                        columns, rows = min(width, 100-x*width), min(width, 100-y*width)
                        key = columns, rows
                        if key not in atlases:
                            atlases[key] = atlas_png(normal, columns, rows)
                        db.execute('INSERT OR REPLACE INTO hierarchy_blobs VALUES(?,?,?,?,?,?)',
                                   (region, level, x, y, 'mesh', group_mesh(mesh, x, y, size=width)))
                        db.execute('INSERT OR REPLACE INTO hierarchy_blobs VALUES(?,?,?,?,?,?)',
                                   (region, level, x, y, 'normal', atlases[key]))
                db.commit()
                print(region, level, width, flush=True)
        manifest['hierarchy'] = {'depths': list(range(8)), 'lodLevels': list(range(7, -1, -1)), 'widths': [2**n for n in range(8)],
                                 'baseCount': 100, 'normalResolution': 32, 'tolerance': 1000}
        db.execute('UPDATE metadata SET value=? WHERE key="manifest"', (json.dumps(manifest, ensure_ascii=False),))


if __name__ == '__main__':
    import sys
    if '--normals-only' in sys.argv:
        generate_normals()
    else:
        generate()
        generate_normals()
