"""抽出した領域や XYZ タイルから、再現可能な GeoJSON の提供範囲を作る。"""

import json
import math


def geometry_json(geometry):
    """同一直線上の余分な頂点を除き、外周を反時計回りにそろえる。"""
    from shapely.geometry import mapping, MultiPolygon, Polygon
    from shapely.geometry.polygon import orient
    if geometry.is_empty or geometry.geom_type not in ("Polygon", "MultiPolygon") or not geometry.is_valid:
        raise ValueError("提供範囲は空でない有効な面にしてください")
    geometry = geometry.simplify(0, preserve_topology=True).normalize()
    polygons = [geometry] if geometry.geom_type == "Polygon" else list(geometry.geoms)
    # 世界全体などの長い辺には中間点を挿入し、日付変更線を横断する辺と区別する。
    def ring(coords):
        result = []
        for a, b in zip(coords, coords[1:]):
            count = max(1, math.ceil(abs(b[0] - a[0]) / 180))
            result.extend((a[0] + (b[0] - a[0]) * i / count,
                           a[1] + (b[1] - a[1]) * i / count) for i in range(count))
        return [*result, result[0]]
    polygons = [orient(Polygon(ring(list(p.exterior.coords)),
                              [ring(list(h.coords)) for h in p.interiors]), sign=1) for p in polygons]
    result = polygons[0] if len(polygons) == 1 else MultiPolygon(polygons)
    return json.loads(json.dumps(mapping(result)))


def union_geometry(geometries):
    from shapely.geometry import shape
    from shapely import union_all
    return geometry_json(union_all([shape(g) for g in geometries]))


def tile_geometry(tiles):
    """XYZ タイルの和集合。ズームの異なるタイル、穴、離れた領域も扱う。"""
    from shapely.geometry import box
    from shapely import transform, union_all
    import numpy as np
    cells = set()
    for z, x, y in tiles:
        if type(z) is not int or not 0 <= z <= 30 or type(x) is not int or type(y) is not int or not 0 <= x < 2 ** z or not 0 <= y < 2 ** z:
            raise ValueError("提供範囲の XYZ タイル座標が不正です")
        cells.add((z, x, y))
    # 全てそろった子タイルは親にまとめる。地理的な範囲は変わらない。
    for z in range(max((t[0] for t in cells), default=0), 0, -1):
        parents = {(x // 2, y // 2) for level, x, y in cells if level == z}
        for x, y in parents:
            children = {(z, x * 2 + dx, y * 2 + dy) for dx in (0, 1) for dy in (0, 1)}
            if children <= cells:
                cells.difference_update(children)
                cells.add((z - 1, x, y))
    # 祖先に含まれるタイルは図形の結合前に除く。
    cells = {t for t in cells if not any((z, t[1] >> (t[0] - z), t[2] >> (t[0] - z)) in cells for z in range(t[0]))}
    rows = {}
    for z, x, y in cells:
        rows.setdefault((z, y), []).append(x)
    rectangles = []
    for (z, y), xs in sorted(rows.items()):
        xs.sort()
        start = end = xs[0]
        for x in [*xs[1:], None]:
            if x is not None and x == end + 1:
                end = x
                continue
            side = 2 ** z
            rectangles.append(box(start / side, y / side, (end + 1) / side, (y + 1) / side))
            start = end = x
    if not rectangles:
        raise ValueError("提供範囲のタイルがありません")
    def lonlat(coordinates):
        return np.array([(x * 360 - 180, math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * y))))) for x,y in coordinates])
    return geometry_json(transform(union_all(rectangles).simplify(0, preserve_topology=True),lonlat))


def parse_poly(content):
    """Geofabrik の .poly の外周・穴を面に変換する。"""
    from shapely.geometry import Polygon
    from shapely import union_all
    lines = iter(content.decode("utf-8").splitlines())
    if not next(lines, "").strip():
        raise ValueError("境界ファイルの名前がありません")
    outer, holes, names = [], [], set()
    ended = False
    for line in lines:
        name = line.strip()
        if not name:
            continue
        if name == "END":
            ended = True
            break
        if name in names:
            raise ValueError("境界ファイルのリング名が重複しています")
        names.add(name)
        coordinates = []
        for row in lines:
            if row.strip() == "END":
                break
            values = row.split()
            if len(values) != 2:
                raise ValueError("境界ファイルの座標が不正です")
            point = tuple(map(float, values))
            if not all(math.isfinite(v) for v in point) or not -180 <= point[0] <= 180 or not -90 <= point[1] <= 90:
                raise ValueError("境界ファイルの座標が範囲外です")
            coordinates.append(point)
        else:
            raise ValueError("境界ファイルのリングが閉じられていません")
        if len(coordinates) < 3:
            raise ValueError("境界ファイルの頂点が足りません")
        polygon = Polygon(coordinates)
        if not polygon.is_valid or polygon.area <= 0:
            raise ValueError("境界ファイルの面が不正です")
        (holes if name.startswith("!") else outer).append(polygon)
    if not ended or not outer or any(line.strip() for line in lines):
        raise ValueError("境界ファイルの終端または外周が不正です")
    polygon = union_all(outer)
    if holes:
        excluded = union_all(holes)
        if not polygon.covers(excluded):
            raise ValueError("境界ファイルの穴が外周からはみ出しています")
        polygon = polygon.difference(excluded)
    return geometry_json(polygon)


def check_points(geometry, points):
    """補完で追加した地点も、元資料の対象範囲に含まれることを確認する。"""
    from shapely.geometry import shape, Point
    from shapely.prepared import prep
    area = prep(shape(geometry))
    outside = [p["id"] for p in points if not area.covers(Point(p["longitude"], p["latitude"]))]
    if outside:
        raise ValueError("元資料の提供範囲に含まれない地点があります。対象範囲の資料を追加してください: " + ", ".join(outside[:10]))
