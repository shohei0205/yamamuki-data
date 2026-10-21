"""共有予算の比較データについて面積・頂点予算・全共有辺を検証する。"""
import json, sqlite3, struct, zlib
from pathlib import Path
import numpy as np
from national_independent_mesh import ROOT


def verify(mode):
    with sqlite3.connect(ROOT / f'build/terrain-shared-allocation-{mode}.sqlite3') as db:
        manifest = json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
        allocation = manifest['sharedAllocation']
        assert allocation['sourceVertices'] <= allocation['totalBudget']
        parts = {}
        for l, x, y, geometry, hints, mask, stats in db.execute('SELECT * FROM tiles'):
            raw = zlib.decompress(geometry); magic, n, ny, nv, ni = struct.unpack_from('<4sHHII', raw)
            assert magic == b'YNT3' and n == ny == 257
            assert len(raw) == 16 + nv * 16 + ni * 2
            p = np.frombuffer(raw, '<f4', nv * 3, 16).reshape(-1, 3).astype(float)
            f = np.frombuffer(raw, '<u2', ni, 16 + nv * 12).reshape(-1, 3)
            ranks = np.frombuffer(raw, '<f4', nv, 16 + nv * 12 + ni * 2)
            assert np.isfinite(p).all() and np.isfinite(ranks).all()
            assert ((ranks == -1) | ((ranks >= 0) & (ranks <= 1))).all()
            assert f.max() < nv and (p[:, :2] >= 0).all() and (p[:, :2] <= 8000).all()
            tri = p[f, :2]; area = np.linalg.det(np.stack([tri[:, 1] - tri[:, 0], tri[:, 2] - tri[:, 0]], axis=2)) / 2
            assert (area > 0).all() and abs(area.sum() - 8000 ** 2) < .05
            assert len(zlib.decompress(mask)) == (257 ** 2 + 7) // 8
            parts[x, y] = p
        shared = 0
        for (x, y), p in parts.items():
            for dx, dy, axis in [(1, 0, 0), (0, 1, 1)]:
                if (x + dx, y + dy) not in parts: continue
                other = parts[x + dx, y + dy]
                a = {tuple(v[[1-axis, 2]]) for v in p if v[axis] == 8000}
                b = {tuple(v[[1-axis, 2]]) for v in other if v[axis] == 0}
                assert a == b, (mode, x, y, axis)
                shared += 1
        divisions=round(allocation.get('widthMeters',32000)/8000);areas=len(manifest['reliefRegions'])
        assert len(parts) == areas*divisions**2 and shared == areas*2*divisions*(divisions-1)
        return dict(mode=mode, tiles=len(parts), sharedEdges=shared, totalSource=allocation['sourceVertices'], regions={a['id']: a['levels'][0]['priorityMetrics'] for a in manifest['reliefRegions']})


if __name__ == '__main__':
    report = [verify(mode) for mode in ['relative', 'balanced', 'terrain', 'terrain128', 'terrain128-b48000', 'terrain128-b96000', 'terrain128-b384000', 'terrain128-b768000'] if (ROOT / f'build/terrain-shared-allocation-{mode}.sqlite3').exists()]
    (ROOT / 'build/terrain-shared-allocation-report.json').write_text(json.dumps(report, indent=2), encoding='utf-8')
    print(json.dumps(report, ensure_ascii=True))
