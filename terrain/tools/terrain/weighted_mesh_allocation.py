"""タイルから独立した頂点予算の評価と、元の面を保つタイル切り分け。"""
import numpy as np


def mixed_limits(relative):
    # 絶対誤差を主にし、小さい山の重みは最大1.25倍に抑える。
    inverse = .7 / 50. + .3 / np.clip(relative, 10., 100.)
    return 1. / np.minimum(inverse, 1.25 / 50.)


def allocation_score(error, relative, valid, terrain_weight=None):
    """同面積の標高格子で残差を積分。追加による実際の減少量ではない。"""
    e = np.maximum(np.abs(error) - 2., 0.)
    values = .7 * e / 50. + .3 * np.minimum(e / np.clip(relative, 10., 100.), 4.)
    base = float(np.where(valid, values, 0.).mean())
    if terrain_weight is None or not valid.any():
        return base
    weights = np.where(valid, np.clip(terrain_weight, 0., 1.), 0.)
    # 山地が少ない地域も評価するが、狭い一点へ予算を集中させない。
    denominator = max(float(weights.sum()), .1 * int(valid.sum()))
    return base + .5 * float((values * weights).sum()) / denominator


def split_mesh(points, faces, width=32000., tile_width=8000.):
    """三角形を長方形でクリップし、共有交点の高さを一度だけ確定する。"""
    points = np.asarray(points, dtype=float)
    result = {}
    heights = {}
    divisions = round(width / tile_width)
    for face in faces:
        tri = points[face]
        lo = np.maximum(0, np.floor(tri[:, :2].min(axis=0) / tile_width).astype(int))
        hi = np.minimum(divisions - 1, np.floor(tri[:, :2].max(axis=0) / tile_width).astype(int))
        basis = np.column_stack([tri[1, :2] - tri[0, :2], tri[2, :2] - tri[0, :2]])
        inverse = np.linalg.inv(basis)
        for tx in range(lo[0], hi[0] + 1):
            for ty in range(lo[1], hi[1] + 1):
                polygon = [v.copy() for v in tri[:, :2]]
                for axis, bound, sign in [(0, tx * tile_width, 1), (0, (tx + 1) * tile_width, -1), (1, ty * tile_width, 1), (1, (ty + 1) * tile_width, -1)]:
                    output = []
                    for a, b in zip(polygon, polygon[1:] + polygon[:1]):
                        inside_a, inside_b = sign * (a[axis] - bound) >= 0, sign * (b[axis] - bound) >= 0
                        if inside_a:
                            output.append(a)
                        if inside_a != inside_b:
                            t = (bound - a[axis]) / (b[axis] - a[axis])
                            q = a + t * (b - a)
                            q[axis] = bound
                            output.append(q)
                    polygon = output
                    if not polygon:
                        break
                ids = []
                tile = result.setdefault((tx, ty), dict(points=[], faces=[], lookup={}))
                for xy in polygon:
                    # 2進数で表現できる座標に揃え、Float32格納後も共有辺を一致させる。
                    key = tuple(np.rint(xy * 256).astype(int))
                    if key not in heights:
                        uv = inverse @ (np.asarray(key) / 256. - tri[0, :2])
                        z = tri[0, 2] + uv @ (tri[1:, 2] - tri[0, 2])
                        heights[key] = round(float(z) * 1024) / 1024
                    if key not in tile['lookup']:
                        tile['lookup'][key] = len(tile['points'])
                        tile['points'].append([key[0] / 256 - tx * tile_width, key[1] / 256 - ty * tile_width, heights[key]])
                    index = tile['lookup'][key]
                    if not ids or ids[-1] != index:
                        ids.append(index)
                if len(ids) > 1 and ids[0] == ids[-1]:
                    ids.pop()
                for i in range(1, len(ids) - 1):
                    f = [ids[0], ids[i], ids[i + 1]]
                    a, b, c = np.asarray([tile['points'][v][:2] for v in f])
                    cross = float(np.linalg.det(np.column_stack([b - a, c - a])))
                    if abs(cross) > 1e-8:
                        tile['faces'].append(f if cross > 0 else f[::-1])
    return {k: dict(points=np.asarray(t['points']), faces=np.asarray(t['faces'], dtype=np.uint32)) for k, t in result.items() if t['faces']}
