"""Audit independently generated WorldCover work areas along their shared z8 edges."""
import argparse
from functools import lru_cache
import json
from pathlib import Path
import sqlite3
import zlib

from shapely import LineString, MultiLineString, from_wkb

WORLD = 40_075_016.68557849
SIDE = WORLD / 256
CLASSES = ('forest', 'bare', 'water')


def line_spans(geometry, axis, origin):
    if geometry.is_empty:
        return
    if geometry.geom_type == 'LineString':
        start, end = geometry.bounds[axis] + origin, geometry.bounds[axis + 2] + origin
        if end > start:
            yield [(start, 0), (end, 0)]
    elif hasattr(geometry, 'geoms'):
        for part in geometry.geoms:
            yield from line_spans(part, axis, origin)


def audit(root, threshold):
    areas = {
        (tile['x'], tile['y']): marker.parent
        for marker in root.glob('*/finished.json')
        for tile in [json.loads(marker.read_bytes())]
    }

    @lru_cache(maxsize=16)
    def read(key, level):
        candidates = [
            path / 'majority-clean' for path in areas[key].glob(f'l{level}*')
            if (path.name == f'l{level}' or path.name.startswith(f'l{level}-retry-'))
            and (path / 'manifest.json').is_file()
        ]
        source = max(candidates)
        meta = json.loads((source / 'manifest.json').read_bytes())
        with sqlite3.connect((source / 'landcover-vectors.sqlite3').resolve().as_uri() + '?mode=ro', uri=True) as db:
            rows = db.execute(
                'SELECT class, geometry FROM features WHERE level=? AND class BETWEEN 1 AND 3 ORDER BY class',
                (level,),
            ).fetchall()
        if [code for code, _ in rows] != [1, 2, 3]:
            raise ValueError(f'Missing classes in {source}')
        return (meta['bounds'][0], -meta['bounds'][3]), [from_wkb(zlib.decompress(blob)) for _, blob in rows]

    levels = []
    for level in range(3, 7):
        edges = []
        for x, y in sorted(areas):
            for dx, dy in ((1, 0), (0, 1)):
                neighbor = x + dx, y + dy
                if neighbor not in areas:
                    continue
                a = (x + dx) * SIDE - WORLD / 2, (y + dy) * SIDE - WORLD / 2
                b = a[0] + dy * SIDE, a[1] + dx * SIDE
                traces = []
                for key in ((x, y), neighbor):
                    (ox, oy), layers = read(key, level)
                    edge = LineString([(a[0] - ox, a[1] - oy), (b[0] - ox, b[1] - oy)])
                    axis = 1 if dx else 0
                    origin = (ox - a[0], oy - a[1])[axis]

                    forest, bare, water = [MultiLineString(list(line_spans(g.intersection(edge), axis, origin))) for g in layers]
                    traces.append([forest.difference(water).difference(bare), bare.difference(water), water])
                mismatch = [left.symmetric_difference(right).length for left, right in zip(*traces)]
                edges.append({'from': [x, y], 'to': neighbor, 'mismatchM': dict(zip(CLASSES, mismatch))})
        levels.append({
            'level': level,
            'edgesChecked': len(edges),
            'edgesOverThreshold': {
                name: sum(edge['mismatchM'][name] > threshold for edge in edges) for name in CLASSES
            },
            'edges': [edge for edge in edges if max(edge['mismatchM'].values()) > threshold],
        })
    return {'completeRegions': len(areas), 'thresholdM': threshold, 'visibleLayersOnly': True, 'levels': levels}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    parser.add_argument('--threshold', type=float, default=1.)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    report = audit(args.root, args.threshold)
    if args.output:
        args.output.write_text(json.dumps(report, indent=2), encoding='utf-8')
    print(json.dumps({
        **report, 'levels': [{k: v for k, v in level.items() if k != 'edges'} for level in report['levels']],
    }, indent=2))
