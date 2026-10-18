"""山の周囲だけを取得し、保存済み自然地名の元ファイルも再利用する。"""

from contextlib import closing
import gzip
import hashlib
import json
import math
from pathlib import Path
import sqlite3

from fetch_all import bounds, fetch, restore_metadata
from generate import NATURAL_URL, read_natural, tile_position


def target_tiles(points, radius=100):
    if not 0 <= radius <= 1000:
        raise ValueError("自然地名の取得半径は 0～1000 m にしてください")
    tiles = set()
    for p in points:
        lat, lon = p["latitude"], p["longitude"]
        angular = radius / 6371000
        dy = math.degrees(angular)
        dx = math.degrees(math.asin(math.sin(angular) / math.cos(math.radians(lat))))
        west, north = tile_position(lat + dy, lon - dx, 15)
        east, south = tile_position(lat - dy, lon + dx, 15)
        tiles.update((x, y) for x in range(math.floor(west), math.floor(east) + 1)
                     for y in range(math.floor(north), math.floor(south) + 1))
    return tiles


def open_cache(path):
    db = sqlite3.connect(path)
    db.execute("CREATE TABLE IF NOT EXISTS responses(x INTEGER,y INTEGER,status INTEGER,body BLOB,metadata TEXT,PRIMARY KEY(x,y))")
    db.execute("CREATE TABLE IF NOT EXISTS info(key TEXT PRIMARY KEY,value TEXT NOT NULL)")
    return db


def acquire(points, destination, inherited=None, offline=False, limit=100000, radius=100):
    """従来の全国 DB は読み取り専用で開き、必要な応答を新しい小さな DB に保存する。"""
    targets = target_tiles(points, radius)
    selected = set(targets)
    original, area, national_complete = None, None, False
    with closing(open_cache(destination)) as saved:
        selected.update(saved.execute("SELECT x,y FROM responses WHERE status=200"))
        national_complete = saved.execute("SELECT value FROM info WHERE key='includesCompletedScan'").fetchone() == ('true',)
    if inherited is not None:
        original = sqlite3.connect(Path(inherited).resolve().as_uri() + "?mode=ro", uri=True)
        config = json.loads(original.execute("SELECT value FROM config WHERE key='scan'").fetchone()[0])
        if config["zoom"] != 15 or config["urlTemplate"] != NATURAL_URL:
            original.close()
            raise ValueError("保存済み自然地名のタイル設定が異なります")
        area = bounds(config["bbox"])
        height = area[3] - area[1] + 1
        # 存在が分かっているタイルだけを再利用し、全国の空白領域へアクセスしない。
        selected.update((area[0] + number // height, area[1] + number % height)
                        for number, in original.execute("SELECT number FROM tiles WHERE status=200"))
        national_complete |= original.execute("SELECT count(*) FROM tiles").fetchone()[0] == (area[2] - area[0] + 1) * height
    if len(selected) > limit:
        if original:
            original.close()
        raise ValueError(f"自然地名の取得対象 {len(selected)} タイルが上限 {limit} を超えます")
    fetched, reused, fetched_missing = 0, 0, 0
    try:
        with closing(open_cache(destination)) as db:
            try:
                for i, (x, y) in enumerate(sorted(selected), 1):
                    row = db.execute("SELECT status,body,metadata FROM responses WHERE x=? AND y=?", (x, y)).fetchone()
                    if row is None and original is not None and area[0] <= x <= area[2] and area[1] <= y <= area[3]:
                        number = (x - area[0]) * (area[3] - area[1] + 1) + y - area[1]
                        row = original.execute("SELECT status,body,metadata FROM tiles WHERE number=?", (number,)).fetchone()
                        if row is not None:
                            metadata = restore_metadata(original, number, row[0], row[2], area)
                            row = (row[0], row[1], json.dumps(metadata, ensure_ascii=False))
                    if row is None:
                        if offline:
                            raise FileNotFoundError(f"自然地名の元データがありません: {NATURAL_URL.format(x=x,y=y)}")
                        _, status, body, metadata = fetch(0, (x, y, x, y))
                        row = status, body, metadata
                        fetched += 1
                        fetched_missing += status == 404
                    else:
                        reused += 1
                    validate(row, x, y)
                    db.execute("INSERT OR IGNORE INTO responses VALUES(?,?,?,?,?)", (x, y, *row))
                    if i % 100 == 0:
                        db.commit()
                    if i % 10000 == 0 or i == len(selected):
                        print(f"自然地名の選択取得: {i}/{len(selected)} タイル（通信 {fetched}、再利用 {reused}）", flush=True)
                positive = sum(db.execute("SELECT status FROM responses WHERE x=? AND y=?", tile).fetchone()[0] == 200 for tile in selected)
                db.execute("INSERT OR REPLACE INTO info VALUES('includesCompletedScan',?)", ('true' if national_complete else 'false',))
            finally:
                db.commit()
    finally:
        if original is not None:
            original.close()
    return selected, {"targetTileCount": len(targets), "selectedTileCount": len(selected), "existingTiles": positive,
                      "missingTiles": len(selected) - positive, "networkRequests": fetched, "network404Count": fetched_missing, "reusedTiles": reused,
                      "radiusM": radius, "includesCompletedScan": national_complete,
                      "coverage": "ベクトル山名・山岳一覧の周囲と、保存済み全国 DB の存在確認済みタイル。新規の全国総当たりは行わない。"}


def validate(row, x, y):
    status, compressed, metadata_json = row
    metadata = json.loads(metadata_json)
    if status not in (200, 404) or metadata["status"] != status or metadata["url"] != NATURAL_URL.format(x=x, y=y):
        raise ValueError("自然地名の保存記録とタイル位置が一致しません")
    if status == 200 and hashlib.sha256(gzip.decompress(compressed)).hexdigest() != metadata["sha256"]:
        raise ValueError("自然地名の元データのハッシュが一致しません")


def load(database, selected):
    peaks, features, skipped, sources = [], [], [], []
    with closing(open_cache(database)) as db:
        for x, y in sorted(selected):
            row = db.execute("SELECT status,body,metadata FROM responses WHERE x=? AND y=?", (x, y)).fetchone()
            validate(row, x, y)
            if row[0] == 404:
                continue
            payload, metadata = gzip.decompress(row[1]), json.loads(row[2])
            points, omissions = read_natural(payload)
            peaks.extend(points)
            skipped.extend({**item, "sourceUrl": metadata["url"]} for item in omissions)
            sources.append(metadata)
            for f in json.loads(payload)["features"]:
                if f["geometry"]["type"] == "Point" and f["properties"].get("gaijiFlg", "0") == "0":
                    lon, lat = f["geometry"]["coordinates"]
                    features.append({"properties": f["properties"], "latitude": lat, "longitude": lon, "sourceUrl": metadata["url"]})
    return peaks, features, skipped, sources
