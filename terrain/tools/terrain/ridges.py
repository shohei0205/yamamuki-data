"""小地域の DEM から稜線候補を作り、2 方式を比較するための試作。"""

import argparse
import base64
import gzip
import hashlib
import io
import json
import math
from pathlib import Path
import time
import urllib.error
import urllib.request

import numpy as np
from PIL import Image


TILE_SIZE = 256
RADIUS = 6378137.0
SOURCE = "https://cyberjapandata.gsi.go.jp/xyz/dem_png"
ATTRIBUTION = "地理院タイル（標高タイル）を加工して作成した稜線候補（山むき）"
DIRECTIONS = [(dy, dx) for dy in (-1, 0, 1) for dx in (-1, 0, 1) if dy or dx]


def tile_position(lon, lat, zoom):
    scale = 2 ** zoom
    return ((lon + 180) / 360 * scale,
            (1 - math.asinh(math.tan(math.radians(lat))) / math.pi) / 2 * scale)


def lon_lat(px, py, zoom):
    scale = TILE_SIZE * 2 ** zoom
    return (px / scale * 360 - 180,
            math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * py / scale)))))


def decode_png(data):
    with Image.open(io.BytesIO(data)) as image:
        if image.size != (TILE_SIZE, TILE_SIZE) or image.mode != "RGB":
            raise ValueError("標高タイルは 256 × 256 の RGB 画像が必要です")
        rgb = np.asarray(image, dtype=np.int32)
    value = rgb[:, :, 0] * 65536 + rgb[:, :, 1] * 256 + rgb[:, :, 2]
    height = np.where(value < 2 ** 23, value, value - 2 ** 24) * 0.01
    height[value == 2 ** 23] = np.nan
    return height


def read_tile(zoom, x, y, cache, offline):
    path = cache / str(zoom) / str(x) / f"{y}.png"
    url = f"{SOURCE}/{zoom}/{x}/{y}.png"
    if not path.exists():
        if offline:
            raise ValueError(f"保存されていない標高タイルです: {path}")
        request = urllib.request.Request(url, headers={"User-Agent": "yamamuki-terrain-prototype/1"})
        # 取得は直列で、応答を保存してから次へ進む。欠損を海抜 0 m に置き換えない。
        for attempt in range(3):
            try:
                with urllib.request.urlopen(request, timeout=30) as response:
                    data = response.read()
                decode_png(data)
                path.parent.mkdir(parents=True, exist_ok=True)
                temporary = path.with_suffix(".tmp")
                temporary.write_bytes(data)
                temporary.replace(path)
                break
            except urllib.error.HTTPError as error:
                if error.code not in (429, 500, 502, 503, 504) or attempt == 2:
                    raise
                time.sleep(2 ** attempt)
            except urllib.error.URLError:
                if attempt == 2:
                    raise
                time.sleep(2 ** attempt)
        time.sleep(0.2)
    data = path.read_bytes()
    return decode_png(data), {"url": url, "sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}


def load_dem(bbox, zoom, cache, offline):
    west, south, east, north = bbox
    if not (-180 < west < east < 180 and -85 < south < north < 85):
        raise ValueError("範囲は西経度・南緯度・東経度・北緯度の順で指定してください")
    left, top = tile_position(west, north, zoom)
    right, bottom = tile_position(east, south, zoom)
    # 表示範囲の周囲 1 タイルを含めて解析し、最後に表示範囲へ切り出す。
    x0, y0 = math.floor(left) - 1, math.floor(top) - 1
    x1, y1 = math.ceil(right) + 1, math.ceil(bottom) + 1
    if x0 < 0 or y0 < 0 or x1 > 2 ** zoom or y1 > 2 ** zoom:
        raise ValueError("周辺タイルを含む範囲が世界の端を越えています")
    count = (x1 - x0) * (y1 - y0)
    if count > 64:
        raise ValueError(f"小地域の試作では 64 タイルまでです（指定は {count} タイル）")
    dem = np.empty(((y1 - y0) * TILE_SIZE, (x1 - x0) * TILE_SIZE))
    sources = []
    for y in range(y0, y1):
        for x in range(x0, x1):
            tile, info = read_tile(zoom, x, y, cache, offline)
            sources.append(info)
            dem[(y-y0)*256:(y-y0+1)*256, (x-x0)*256:(x-x0+1)*256] = tile
    origin = (x0 * TILE_SIZE, y0 * TILE_SIZE)
    crop = (math.ceil(left * 256 - origin[0]), math.ceil(top * 256 - origin[1]),
            math.floor(right * 256 - origin[0]), math.floor(bottom * 256 - origin[1]))
    if crop[2] - crop[0] < 4 or crop[3] - crop[1] < 4:
        raise ValueError("表示範囲が小さすぎます")
    return dem, origin, crop, sources


def smooth_dem(dem, passes):
    dem = np.asarray(dem, dtype=float)
    result = dem.copy()
    for _ in range(passes):
        padded = np.pad(result, 1, constant_values=np.nan)
        total = np.zeros_like(result)
        count = np.zeros_like(result)
        for dy in range(3):
            for dx in range(3):
                part = padded[dy:dy+dem.shape[0], dx:dx+dem.shape[1]]
                valid = np.isfinite(part)
                total += np.where(valid, part, 0)
                count += valid
        result = np.divide(total, count, out=np.full_like(total, np.nan), where=count > 0)
        result[~np.isfinite(dem)] = np.nan
    return result


def flow_routes(dem):
    """D8 の最急降下先。平坦面と欠損は終点にし、循環を作らない。"""
    dem = np.asarray(dem, dtype=float)
    height, width = dem.shape
    index = np.arange(dem.size).reshape(dem.shape)
    route = index.copy()
    best = np.zeros_like(dem)
    padded = np.pad(dem, 1, constant_values=np.nan)
    for dy, dx in DIRECTIONS:
        neighbor = padded[1+dy:1+dy+height, 1+dx:1+dx+width]
        slope = (dem - neighbor) / math.hypot(dx, dy)
        take = np.isfinite(slope) & (slope > best)
        route[take] = (index + dy * width + dx)[take]
        best[take] = slope[take]
    return route.ravel()


def drainage_basins(dem):
    route = flow_routes(dem)
    labels = route.copy()
    # 厳密に低い点への経路なので、低い順に終点のラベルが確定する。
    for cell in np.argsort(dem.ravel(), kind="stable"):
        labels[cell] = labels[route[cell]]
    return labels.reshape(dem.shape)


def watershed_edges(dem, min_basin_cells):
    """異なる流域の境界を画素の辺で取り出す。座標は画素中心基準。"""
    labels = drainage_basins(dem)
    counts = np.bincount(labels.ravel(), minlength=dem.size)
    eligible = np.isfinite(dem) & (counts[labels] >= min_basin_cells)
    edges = []
    for y, x in np.argwhere((labels[:, :-1] != labels[:, 1:]) & eligible[:, :-1] & eligible[:, 1:]):
        edges.append(((int(x)+0.5, int(y)-0.5), (int(x)+0.5, int(y)+0.5)))
    for y, x in np.argwhere((labels[:-1, :] != labels[1:, :]) & eligible[:-1, :] & eligible[1:, :]):
        edges.append(((int(x)-0.5, int(y)+0.5), (int(x)+0.5, int(y)+0.5)))
    return edges


def inverse_flow_edges(dem, min_cells):
    """標高を反転した流路。鞍部をまたぐ接続は保証しない比較用の候補。"""
    inverted = -dem
    route = flow_routes(inverted)
    accumulation = np.ones(dem.size, dtype=np.int64)
    accumulation[~np.isfinite(dem.ravel())] = 0
    for cell in np.argsort(-inverted.ravel(), kind="stable"):
        target = route[cell]
        if target != cell:
            accumulation[target] += accumulation[cell]
    width = dem.shape[1]
    return [((int(cell % width), int(cell // width)),
             (int(route[cell] % width), int(route[cell] // width)))
            for cell in np.flatnonzero((accumulation >= min_cells) & (route != np.arange(dem.size)))]


def trace_paths(edges):
    neighbors = {}
    remaining = set()
    for a, b in edges:
        edge = tuple(sorted((a, b)))
        if a == b or edge in remaining:
            continue
        remaining.add(edge)
        neighbors.setdefault(a, set()).add(b)
        neighbors.setdefault(b, set()).add(a)

    def walk(a, b):
        path = [a, b]
        remaining.remove(tuple(sorted((a, b))))
        while len(neighbors[b]) == 2:
            c = next(point for point in sorted(neighbors[b]) if point != a)
            edge = tuple(sorted((b, c)))
            if edge not in remaining:
                break
            remaining.remove(edge)
            path.append(c)
            a, b = b, c
        return path

    paths = []
    for a in sorted(neighbors):
        if len(neighbors[a]) != 2:
            for b in sorted(neighbors[a]):
                if tuple(sorted((a, b))) in remaining:
                    paths.append(walk(a, b))
    while remaining:
        paths.append(walk(*min(remaining)))
    return paths


def sample_height(dem, point):
    x, y = point
    x0, y0 = math.floor(x), math.floor(y)
    if x0 < 0 or y0 < 0 or x0+1 >= dem.shape[1] or y0+1 >= dem.shape[0]:
        return float("nan")
    dx, dy = x-x0, y-y0
    weights = np.array([[(1-dx)*(1-dy), dx*(1-dy)], [(1-dx)*dy, dx*dy]])
    values = dem[y0:y0+2, x0:x0+2]
    if np.any((weights > 0) & ~np.isfinite(values)):
        return float("nan")
    return float(np.sum(np.where(weights > 0, values, 0) * weights))


def clip_paths(paths, crop):
    # 境界との交点を共有するため、隣接タイルに切り出す場合も同じ線を入力する。
    left, top, right, bottom = crop
    result = []
    for path in paths:
        current = []
        for a, b in zip(path, path[1:]):
            dx, dy = b[0]-a[0], b[1]-a[1]
            low, high = 0.0, 1.0
            for p, q in [(-dx, a[0]-left), (dx, right-a[0]), (-dy, a[1]-top), (dy, bottom-a[1])]:
                if p == 0:
                    if q < 0:
                        high = -1
                        break
                elif p < 0:
                    low = max(low, q/p)
                else:
                    high = min(high, q/p)
            if low <= high:
                start = (a[0]+low*dx, a[1]+low*dy)
                end = (a[0]+high*dx, a[1]+high*dy)
                if start == end:
                    continue
                if current and current[-1] != start:
                    if len(current) >= 2:
                        result.append(current)
                    current = []
                if not current:
                    current.append(start)
                current.append(end)
            elif current:
                if len(current) >= 2:
                    result.append(current)
                current = []
        if len(current) >= 2:
            result.append(current)
    return result


def simplify(points, xy_error, height_error):
    """水平位置と標高の誤差を別々に制限する。端点・分岐は呼び出し元で固定。"""
    if len(points) <= 2:
        return points
    array = np.array(points)
    keep = {0, len(points)-1}
    stack = [(0, len(points)-1)]
    while stack:
        start, end = stack.pop()
        if end <= start+1:
            continue
        a, b = array[start], array[end]
        delta = b[:2]-a[:2]
        length2 = float(delta @ delta)
        middle = array[start+1:end]
        t = np.clip((middle[:, :2]-a[:2]) @ delta / length2, 0, 1) if length2 else np.zeros(len(middle))
        xy = np.linalg.norm(middle[:, :2]-(a[:2]+t[:, None]*delta), axis=1) / xy_error
        z = np.abs(middle[:, 2]-(a[2]+t*(b[2]-a[2]))) / height_error
        error = np.maximum(xy, z)
        worst = int(np.argmax(error))
        if error[worst] > 1:
            split = start+1+worst
            keep.add(split)
            stack.extend([(start, split), (split, end)])
    return [points[i] for i in sorted(keep)]


def build_lines(dem, paths, origin, zoom, crop, min_length, xy_error, height_error):
    lat = lon_lat(origin[0], origin[1]+dem.shape[0]/2, zoom)[1]
    meters = 2*math.pi*RADIUS*math.cos(math.radians(lat)) / (256*2**zoom)
    result = []
    for path in clip_paths(paths, crop):
        points = [(x*meters, y*meters, sample_height(dem, (x, y))) for x, y in path]
        # 欠損をまたぐ線を作らない。分岐単位の長さによる選別は試作上の制約。
        if not all(math.isfinite(p[2]) for p in points):
            continue
        length = sum(math.hypot(b[0]-a[0], b[1]-a[1]) for a, b in zip(points, points[1:]))
        if length < min_length:
            continue
        simple = simplify(points, xy_error, height_error)
        result.append({"lengthMeters": round(length, 2), "coordinates": [
            [*lon_lat(origin[0]+x/meters, origin[1]+y/meters, zoom), round(z, 2)] for x, y, z in simple]})
    return result, meters


def write_json(path, value):
    data = (json.dumps(value, ensure_ascii=False, separators=(",", ":"), allow_nan=False)+"\n").encode()
    path.write_bytes(data)
    path.with_suffix(path.suffix+".gz").write_bytes(gzip.compress(data, mtime=0))


def run(args):
    started = time.perf_counter()
    dem, origin, crop, sources = load_dem(args.bbox, args.zoom, args.cache, args.offline)
    left, top, right, bottom = crop
    if not np.isfinite(dem[top:bottom+1, left:right+1]).any():
        raise ValueError("表示範囲に有効な標高がありません")
    smoothed = smooth_dem(dem, args.smooth)
    args.output.mkdir(parents=True, exist_ok=True)
    summary = {"schemaVersion": 1, "generatorVersion": 1, "experimental": True, "attribution": ATTRIBUTION,
               "bbox": args.bbox, "parameters": {key: value for key, value in vars(args).items()
                                                 if key not in ("cache", "output", "offline")},
               "sources": sources, "methods": {}}
    all_lines = {}
    for method, extractor in [("watershed", watershed_edges), ("inverse-flow", inverse_flow_edges)]:
        method_started = time.perf_counter()
        edges = extractor(smoothed, args.min_cells)
        lines, meters = build_lines(dem, trace_paths(edges), origin, args.zoom, crop,
                                   args.min_length, args.xy_error, args.height_error)
        collection = {"type": "FeatureCollection", "experimental": True, "attribution": ATTRIBUTION,
                      "features": [{"type": "Feature", "id": f"{method}-{i}",
                                    "properties": {"method": method, "lengthMeters": line["lengthMeters"]},
                                    "geometry": {"type": "LineString", "coordinates": line["coordinates"]}}
                                   for i, line in enumerate(lines)]}
        path = args.output / f"{method}.geojson"
        write_json(path, collection)
        summary["methods"][method] = {"lines": len(lines), "vertices": sum(len(l["coordinates"]) for l in lines),
                                       "seconds": round(time.perf_counter()-method_started, 3),
                                       "gzipBytes": path.with_suffix(".geojson.gz").stat().st_size,
                                       "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
        all_lines[method] = lines
    # 元の解像度を保存する。表示用メッシュの間引きはプレビュー側だけで行う。
    terrain = dem[top:bottom+1, left:right+1]
    metadata = {"schemaVersion": 1, "originPixel": [origin[0]+left, origin[1]+top], "zoom": args.zoom,
                "sampleLocation": "pixel", "shape": list(terrain.shape), "attribution": ATTRIBUTION}
    np.savez_compressed(args.output / "terrain.npz", heights=terrain.astype(np.float32),
                        metadata=json.dumps(metadata, ensure_ascii=False))
    summary["terrain"] = {**metadata, "bytes": (args.output / "terrain.npz").stat().st_size,
                          "sha256": hashlib.sha256((args.output / "terrain.npz").read_bytes()).hexdigest(),
                          "missingSamples": int(np.count_nonzero(~np.isfinite(terrain))),
                          "approximateSampleSpacingMeters": round(meters, 2)}
    viewer = Path(__file__).with_name("viewer.html").read_text(encoding="utf-8")
    # 同じ領域の表示用メッシュを両方式に使い、線以外の色は描かない。
    stride = args.mesh_step
    rows = list(range(0, terrain.shape[0]-1, stride)) + [terrain.shape[0]-1]
    columns = list(range(0, terrain.shape[1]-1, stride)) + [terrain.shape[1]-1]
    center_lon, center_lat = lon_lat(origin[0]+(left+right)/2, origin[1]+(top+bottom)/2, args.zoom)
    center_height = float(np.nanmedian(terrain)) if np.isfinite(terrain).any() else 0

    def local(coordinate):
        lon, lat, z = coordinate
        return [(lon-center_lon)*math.pi/180*RADIUS*math.cos(math.radians(center_lat)),
                (lat-center_lat)*math.pi/180*RADIUS, z-center_height]

    # 標高だけを圧縮して埋め込み、頂点と三角形はブラウザーで復元する。
    compressed = gzip.compress(terrain.astype("<f4").tobytes(), mtime=0)
    payload = {"attribution": ATTRIBUTION,
               "heightfield": {**metadata, "encoding": "gzip-float32-le-base64",
                               "values": base64.b64encode(compressed).decode("ascii"),
                               "center": [center_lon, center_lat, center_height], "step": stride},
               "meshStepMeters": meters*stride,
               "span": max((right-left)*meters, (bottom-top)*meters),
               "methods": {name: [[local(p) for p in line["coordinates"]] for line in lines]
                           for name, lines in all_lines.items()}}
    # HTML はデータを埋め込み、file:// で通信せずに開ける。
    embedded = json.dumps(payload, ensure_ascii=False, allow_nan=False).replace("<", "\\u003c")
    (args.output / "preview.html").write_text(viewer.replace("/*DATA*/", embedded), encoding="utf-8", newline="\n")
    summary["totalSeconds"] = round(time.perf_counter()-started, 3)
    summary["previewMeshStep"] = stride
    write_json(args.output / "report.json", summary)
    print(json.dumps(summary, ensure_ascii=False, indent=2))


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--bbox", type=float, nargs=4, required=True, metavar=("WEST", "SOUTH", "EAST", "NORTH"))
    result.add_argument("--zoom", type=int, default=14, choices=[14])
    result.add_argument("--cache", type=Path, default=Path("build/terrain-cache"))
    result.add_argument("--output", type=Path, default=Path("build/terrain-prototype"))
    result.add_argument("--offline", action="store_true")
    result.add_argument("--smooth", type=int, default=2)
    result.add_argument("--min-cells", type=int, default=100)
    result.add_argument("--min-length", type=float, default=100)
    result.add_argument("--xy-error", type=float, default=10)
    result.add_argument("--height-error", type=float, default=2)
    result.add_argument("--mesh-step", type=int, default=4)
    return result


def main():
    arguments = parser()
    args = arguments.parse_args()
    if not all(math.isfinite(value) for value in args.bbox):
        arguments.error("範囲には有限の数値を指定してください")
    if not 0 <= args.smooth <= 10 or args.min_cells < 1 or args.mesh_step < 1:
        arguments.error("平滑化は 0〜10、セル数とメッシュの間隔は 1 以上にしてください")
    if not all(math.isfinite(value) and value > 0 for value in (args.min_length, args.xy_error, args.height_error)):
        arguments.error("長さと誤差には正の有限の数値を指定してください")
    try:
        run(args)
    except (ValueError, OSError, urllib.error.URLError) as error:
        arguments.exit(1, f"地形データの試作に失敗しました: {error}\n")


if __name__ == "__main__":
    main()
