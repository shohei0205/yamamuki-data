"""山頂付近の標高注記を照合し、欠けた標高を高解像度 DEM から補う。"""

from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor, as_completed
from contextlib import closing
import gzip
import hashlib
import io
import json
import math
import mmap
from pathlib import Path
import re
import sqlite3

from PIL import Image

from generate import distance, tile_position

DEM_LAYERS = (("dem1a_png", 17), ("dem5a_png", 15), ("dem5b_png", 15),
              ("dem5c_png", 15), ("dem_png", 14))
HEIGHT_CODES = {7102, 7201}
MATCH_RADIUS = 25


def height_features(payload, x, y):
    from mapbox_vector_tile.Mapbox.vector_tile_pb2 import tile
    message = tile()
    message.ParseFromString(gzip.decompress(payload) if payload[:2] == b"\x1f\x8b" else payload)
    result = []
    for layer in message.layers:
        if layer.name != "Anno":
            continue
        for feature in layer.features:
            tags = list(feature.tags)
            properties = {layer.keys[tags[i]]: layer.values[tags[i+1]].ListFields()[0][1] for i in range(0, len(tags), 2)}
            if properties.get("vt_code") not in HEIGHT_CODES:
                continue
            geometry = list(feature.geometry)
            if feature.type != 1 or len(geometry) != 3 or geometry[0] != 9:
                raise ValueError("三角点・標高点の注記が単一点ではありません")
            text = str(properties.get("vt_text", "")).strip()
            # 標高の数字を伴わない測量点は照合候補にしない。
            if not text:
                continue
            if not re.fullmatch(r"[+-]?[0-9]+(?:\.[0-9]+)?", text):
                raise ValueError(f"標高注記の値を解釈できません: {text}")
            a, b = [(v >> 1) ^ -(v & 1) for v in geometry[1:]]
            if not (0 <= a < layer.extent and 0 <= b < layer.extent):
                continue
            result.append({"latitude": math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * (y + b / layer.extent) / 2**16)))),
                           "longitude": (x + a / layer.extent) / 2**16 * 360 - 180,
                           "elevationM": float(text), "code": properties["vt_code"], "tile": [16, x, y]})
    return result


def extract_heights(archive, database, points):
    from pmtiles.reader import Reader
    from pmtiles.tile import deserialize_directory, find_tile, zxy_to_tileid
    tiles = set()
    # 境界にある注記を漏らさないよう、照合距離より広い矩形でタイルを選ぶ。
    for p in points:
        lat, lon = p["latitude"], p["longitude"]
        dy = math.degrees((MATCH_RADIUS + 5) / 6371000)
        dx = dy / math.cos(math.radians(lat))
        x1, y1 = tile_position(lat + dy, lon - dx, 16)
        x2, y2 = tile_position(lat - dy, lon + dx, 16)
        tiles.update((x, y) for x in range(int(x1), int(x2)+1) for y in range(int(y1), int(y2)+1))
    identity = json.dumps([str(archive.resolve()), archive.stat().st_size, archive.stat().st_mtime_ns, 1])
    with closing(sqlite3.connect(database)) as db:
        db.execute("CREATE TABLE IF NOT EXISTS config(key TEXT PRIMARY KEY,value TEXT)")
        db.execute("CREATE TABLE IF NOT EXISTS tiles(x INTEGER,y INTEGER,body TEXT,PRIMARY KEY(x,y))")
        saved = db.execute("SELECT value FROM config WHERE key='archive'").fetchone()
        if saved and saved[0] != identity:
            raise ValueError("標高注記の抽出元が異なります。別の抽出 DB を指定してください")
        db.execute("INSERT OR IGNORE INTO config VALUES('archive',?)", (identity,))
        with archive.open("rb") as stream, mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ) as mapping:
            header = Reader(lambda offset, length: mapping[offset:offset+length]).header()
            directories = {}
            def directory(offset, length):
                key = offset, length
                if key not in directories:
                    directories[key] = deserialize_directory(mapping[offset:offset+length])
                return directories[key]
            result = []
            try:
                for i, (x, y) in enumerate(sorted(tiles), 1):
                    saved = db.execute("SELECT body FROM tiles WHERE x=? AND y=?", (x, y)).fetchone()
                    if saved:
                        records = json.loads(saved[0])
                    else:
                        identifier = zxy_to_tileid(16, x, y)
                        entries = directory(header["root_offset"], header["root_length"])
                        records = []
                        for _ in range(4):
                            entry = find_tile(entries, identifier)
                            if entry is None:
                                break
                            if entry.run_length:
                                offset = header["tile_data_offset"] + entry.offset
                                records = height_features(mapping[offset:offset+entry.length], x, y)
                                break
                            entries = directory(header["leaf_directory_offset"] + entry.offset, entry.length)
                        else:
                            raise ValueError("PMTiles のディレクトリが深すぎます")
                        db.execute("INSERT INTO tiles VALUES(?,?,?)", (x, y, json.dumps(records, ensure_ascii=False)))
                    result.extend(records)
                    if i % 1000 == 0:
                        db.commit()
                    if i % 10000 == 0 or i == len(tiles):
                        print(f"標高注記の抽出: {i}/{len(tiles)} タイル", flush=True)
            finally:
                db.commit()
    # 同じ位置・標高の注記が重複していても一つの候補として扱う。
    unique = {}
    for p in result:
        key = p["latitude"], p["longitude"], p["elevationM"]
        unique.setdefault(key, p)
    return list(unique.values())


def apply_annotations(points, measurements):
    buckets = defaultdict(list)
    for i, p in enumerate(measurements):
        buckets[math.floor(p["latitude"]*100), math.floor(p["longitude"]*100)].append(i)
    matches = {}
    for p in points:
        x, y = math.floor(p["latitude"]*100), math.floor(p["longitude"]*100)
        close = []
        for a in range(x-1, x+2):
            for b in range(y-1, y+2):
                for i in buckets[a, b]:
                    d = distance(p, measurements[i])
                    if d <= MATCH_RADIUS:
                        close.append((d, i))
        matches[p["id"]] = sorted(close)
    reverse = Counter(close[0][1] for close in matches.values() if close)
    applied, unresolved = [], []
    for p in points:
        if p["elevationM"] is not None:
            continue
        close = matches[p["id"]]
        if len(close) == 1 and reverse[close[0][1]] == 1:
            d, i = close[0]
            source = measurements[i]
            p["elevationM"] = source["elevationM"]
            applied.append({"id": p["id"], "source": "triangulation" if source["code"] == 7102 else "spotHeight",
                            "distanceM": round(d, 3), "measurement": source})
        else:
            unresolved.append({"id": p["id"], "reason": "候補なし" if not close else "複数候補" if len(close)>1 else "別の山と候補を共有"})
    return {"applied": applied, "unresolved": unresolved, "radiusM": MATCH_RADIUS}


def png_elevation(payload, lat, lon, zoom):
    with Image.open(io.BytesIO(payload)) as image:
        if image.format != "PNG" or image.size != (256, 256) or image.mode != "RGB":
            raise ValueError("標高 PNG は 256 × 256 の RGB 画像である必要があります")
        x, y = tile_position(lat, lon, zoom)
        r, g, b = image.getpixel((int(x % 1 * 256), int(y % 1 * 256)))
    value = r * 65536 + g * 256 + b
    if value == 2**23:
        return None
    return round((value if value < 2**23 else value - 2**24) * .01, 2)


def fill_dem(points, cache, workers=4):
    if not 1 <= workers <= 4:
        raise ValueError("DEM の同時取得数は 1～4 にしてください")
    assignments, counts = [], Counter()
    for kind, zoom in DEM_LAYERS:
        groups = defaultdict(list)
        for p in points:
            if p["elevationM"] is None:
                x, y = tile_position(p["latitude"], p["longitude"], zoom)
                groups[int(x), int(y)].append(p)
        if not groups:
            break
        print(f"{kind}: {len(groups)} タイルを取得・再利用", flush=True)
        with ThreadPoolExecutor(max_workers=workers) as executor:
            futures = {}
            for (x, y), group in groups.items():
                url = f"https://cyberjapandata.gsi.go.jp/xyz/{kind}/{zoom}/{x}/{y}.png"
                futures[executor.submit(cache.get, url, missing_ok=True)] = url, group
            for i, future in enumerate(as_completed(futures), 1):
                url, group = futures.pop(future)
                payload = future.result()
                if payload is not None:
                    for p in group:
                        value = png_elevation(payload, p["latitude"], p["longitude"], zoom)
                        if value is not None:
                            p["elevationM"] = value
                            counts[kind] += 1
                            assignments.append({"id": p["id"], "source": kind, "elevationM": value, "sourceUrl": url})
                if i % 100 == 0 or i == len(groups):
                    print(f"{kind}: {i}/{len(groups)} タイル", flush=True)
    return {"pointCount": sum(counts.values()), "bySource": dict(counts),
            "assignments": sorted(assignments, key=lambda r:r["id"]),
            "missingIds": sorted(p["id"] for p in points if p["elevationM"] is None)}
