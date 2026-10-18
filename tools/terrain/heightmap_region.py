"""全国の保存済み標高から、筑波山の比較用 PNG レベルを生成する。"""
import math
from pathlib import Path
import sqlite3

from heightmap_export import export
from heightmap_levels import build

ROOT = Path(__file__).resolve().parents[2]


def generate():
    lat, lon = 36.2254, 140.1069
    center_x = int((lon+180)/360*2**14)
    center_y = int((1-math.asinh(math.tan(math.radians(lat)))/math.pi)/2*2**14)
    source = ROOT/'build/terrain-tsukuba-preview-source.sqlite3'
    with sqlite3.connect((ROOT/'build/terrain-lod.sqlite3').as_uri()+'?mode=ro', uri=True) as src:
        rows = src.execute('SELECT z,x,y,data FROM tiles WHERE z=14 AND x>=? AND x<? AND y>=? AND y<?',
                           (center_x-8, center_x+8, center_y-8, center_y+8)).fetchall()
    if not rows:
        raise ValueError('筑波山の標高データがありません')
    with sqlite3.connect(source) as db:
        db.execute('CREATE TABLE IF NOT EXISTS tiles(z INTEGER,x INTEGER,y INTEGER,data BLOB,PRIMARY KEY(z,x,y))')
        db.executemany('INSERT OR REPLACE INTO tiles VALUES(?,?,?,?)', rows)
    output = ROOT/'build/terrain-heightmap-tsukuba'
    export(source, output)
    build(output)


if __name__ == '__main__':
    generate()
