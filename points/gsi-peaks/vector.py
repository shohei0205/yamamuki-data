"""最適化ベクトルタイルの山名抽出と、国土地理院の地点の照合を行う。"""

from collections import defaultdict
from contextlib import closing
import gzip
import hashlib
import json
import math
import mmap
from pathlib import Path
import sqlite3
import time
import unicodedata

from generate import distance, point, reading

PMTILES_URL = "https://cyberjapandata.gsi.go.jp/xyz/optimal_bvmap-v1/optimal_bvmap-v1.pmtiles"
# 311 は山の総称であり、山頂として追加しない。低ズームの総描データも使わない。
MOUNTAIN_CODES = {312, 313, 810}


def annotations(payload, z, x, y):
    from mapbox_vector_tile.Mapbox.vector_tile_pb2 import tile
    message = tile()
    message.ParseFromString(gzip.decompress(payload) if payload[:2] == b"\x1f\x8b" else payload)
    points, skipped = [], []
    for layer in message.layers:
        if layer.name != "Anno":
            continue
        for feature in layer.features:
            tags = list(feature.tags)
            if len(tags) % 2:
                raise ValueError("ベクトルタイルの属性が不正です")
            properties = {}
            for i in range(0, len(tags), 2):
                value = layer.values[tags[i + 1]]
                fields = value.ListFields()
                if len(fields) != 1:
                    raise ValueError("ベクトルタイルの属性値が不正です")
                properties[layer.keys[tags[i]]] = fields[0][1]
            if properties.get("vt_code") not in MOUNTAIN_CODES:
                continue
            name = properties.get("vt_text", "").strip()
            geometry = list(feature.geometry)
            # Point 一点の MoveTo だけを採用する。線や複数点を勝手に山頂へ変換しない。
            if feature.type != 1 or len(geometry) != 3 or geometry[0] != 9 or not name:
                skipped.append({"tile": [z, x, y], "properties": properties,
                                "reason": "山名の注記が単一の点ではない、または山名が空"})
                continue
            a, b = [(v >> 1) ^ -(v & 1) for v in geometry[1:]]
            # タイル外のバッファにコピーされた注記は、本来のタイルだけで採用する。
            if not (0 <= a < layer.extent and 0 <= b < layer.extent):
                continue
            lon = (x + a / layer.extent) / 2 ** z * 360 - 180
            lat = math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * (y + b / layer.extent) / 2 ** z))))
            identifier = "gsi-vector:" + hashlib.sha256(f"{name}|{lat:.7f}|{lon:.7f}".encode()).hexdigest()[:24]
            points.append({"point": point(identifier, name, None, lat, lon), "tile": [z, x, y], "properties": properties})
    return points, skipped


def extract(archive, database):
    with closing(sqlite3.connect(database)) as db:
        return extract_database(archive, db)


def extract_database(archive, db):
    from pmtiles.reader import Reader
    from pmtiles.tile import deserialize_directory, tileid_to_zxy
    db.execute("CREATE TABLE IF NOT EXISTS state(key TEXT PRIMARY KEY,value TEXT NOT NULL)")
    db.execute("CREATE TABLE IF NOT EXISTS annotations(id TEXT PRIMARY KEY,body TEXT NOT NULL)")
    db.execute("CREATE TABLE IF NOT EXISTS skipped(tile INTEGER PRIMARY KEY,body TEXT NOT NULL)")
    stat = archive.stat()
    identity = json.dumps([str(archive.resolve()), stat.st_size, stat.st_mtime_ns])
    existing = db.execute("SELECT value FROM state WHERE key='archive'").fetchone()
    if existing and existing[0] != identity:
        raise ValueError("抽出途中の PMTiles と入力が異なります。別の抽出 DB を指定してください")
    db.execute("INSERT OR IGNORE INTO state VALUES('archive',?)", (identity,))
    last = int((db.execute("SELECT value FROM state WHERE key='last'").fetchone() or ["-1"])[0])
    processed = int((db.execute("SELECT value FROM state WHERE key='processed'").fetchone() or ["0"])[0])
    with archive.open("rb") as stream, mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ) as mapping:
        def get_bytes(offset, length):
            return mapping[offset:offset + length]
        header = Reader(get_bytes).header()
        if header["max_zoom"] != 16 or stat.st_size != header["tile_data_offset"] + header["tile_data_length"]:
            raise ValueError("PMTiles が未完了、または想定と異なるズームです")
        def entries(offset, length):
            for entry in deserialize_directory(get_bytes(offset, length)):
                if entry.run_length:
                    yield entry
                else:
                    yield from entries(header["leaf_directory_offset"] + entry.offset, entry.length)
        tick = time.monotonic()
        for entry in entries(header["root_offset"], header["root_length"]):
            if entry.tile_id + entry.run_length - 1 <= last:
                continue
            for identifier in range(max(last + 1, entry.tile_id), entry.tile_id + entry.run_length):
                z, x, y = tileid_to_zxy(identifier)
                if z != 16:
                    continue
                payload = get_bytes(header["tile_data_offset"] + entry.offset, entry.length)
                points, skipped = annotations(payload, z, x, y)
                for record in points:
                    db.execute("INSERT OR IGNORE INTO annotations VALUES(?,?)", (record["point"]["id"], json.dumps(record, ensure_ascii=False)))
                if skipped:
                    db.execute("INSERT OR REPLACE INTO skipped VALUES(?,?)", (identifier, json.dumps(skipped, ensure_ascii=False)))
                processed += 1
                db.execute("INSERT OR REPLACE INTO state VALUES('last',?)", (str(identifier),))
                if processed % 1000 == 0:
                    db.execute("INSERT OR REPLACE INTO state VALUES('processed',?)", (str(processed),))
                    db.commit()
                if time.monotonic() - tick >= 10:
                    count = db.execute("SELECT count(*) FROM annotations").fetchone()[0]
                    print(f"ベクトルタイルの抽出: {processed} タイル、{count} 地点", flush=True)
                    tick = time.monotonic()
        db.execute("INSERT OR REPLACE INTO state VALUES('processed',?)", (str(processed),))
        db.execute("INSERT OR REPLACE INTO state VALUES('complete','true')")
        db.commit()
    records = [json.loads(row[0]) for row in db.execute("SELECT body FROM annotations ORDER BY id")]
    skipped = [item for row in db.execute("SELECT body FROM skipped ORDER BY tile") for item in json.loads(row[0])]
    return records, skipped, processed


def normalized(name):
    value = "".join(unicodedata.normalize("NFKC", name).split())
    return value.translate(str.maketrans({"槇": "槙", "峯": "峰", "嶽": "岳", "ヶ": "ケ", "舘": "館", "寶": "宝"}))


def supplement(original, records):
    result = [dict(p, aliases=list(p["aliases"])) for p in original]
    index = defaultdict(list)
    def add(p):
        for name in {p["name"], *p["aliases"]}:
            index[normalized(name)].append(p)
    for p in result:
        add(p)
    matched, added, review = [], [], []
    for record in records:
        p = record["point"]
        candidates = {a["id"]: a for a in index[normalized(p["name"])]}
        nearby = sorted([(distance(a, p), a) for a in candidates.values() if distance(a, p) <= 1000], key=lambda v: (v[0], v[1]["id"]))
        close = [(d, a) for d, a in nearby if d <= 100]
        if len(close) == 1:
            d, a = close[0]
            matched.append({"vectorId": p["id"], "existingId": a["id"], "distanceM": round(d, 2), **{k: record[k] for k in ("tile", "properties")}})
        elif close:
            review.append({"vector": record, "reason": "100 m 以内に同名・別名の地点が複数ある", "candidates": [{"id": a["id"], "distanceM": round(d, 2)} for d, a in close]})
        elif nearby:
            review.append({"vector": record, "reason": "同名・別名の地点が 100～1000 m にあるため追加を保留", "candidates": [{"id": a["id"], "distanceM": round(d, 2)} for d, a in nearby]})
        else:
            result.append(p.copy())
            add(result[-1])
            added.append(record)
    return sorted(result, key=lambda p: p["id"]), {"matched": matched, "added": added, "review": review}


def natural_readings(points, database):
    """山と判定した注記の読みだけを、山以外の種別も含む保存済み自然地名から補う。"""
    pending = defaultdict(list)
    for p in points:
        if p["nameReading"] is None:
            pending[normalized(p["name"])].append(p)
    candidates = defaultdict(list)
    with closing(sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True)) as db:
        for compressed, metadata_json in db.execute("SELECT body,metadata FROM tiles WHERE status=200"):
            payload = gzip.decompress(compressed)
            metadata = json.loads(metadata_json)
            if hashlib.sha256(payload).hexdigest() != metadata["sha256"]:
                raise ValueError("自然地名の保存済みファイルのハッシュが一致しません")
            for feature in json.loads(payload)["features"]:
                properties, geometry = feature["properties"], feature["geometry"]
                if properties.get("gaijiFlg", "0") != "0" or geometry["type"] != "Point":
                    continue
                for name_key, kana_key in (("name", "kana"), ("Aname", "Akana")):
                    name, kana = properties.get(name_key, ""), properties.get(kana_key, "")
                    key = normalized(name)
                    if key not in pending or not kana:
                        continue
                    values = [reading(value) for value in kana.split(",")]
                    lon, lat = geometry["coordinates"]
                    source_point = point("natural-reading", name, values[0], lat, lon)
                    for p in pending[key]:
                        d = distance(p, source_point)
                        if d <= 100:
                            candidates[p["id"]].append({"name": name, "reading": values[0], "readingVariants": values,
                                                        "distanceM": round(d, 2), "naturalType": properties["type"], "sourceUrl": metadata["url"]})
    applied, review = [], []
    for group in pending.values():
        for p in group:
            matches = candidates[p["id"]]
            if len({r["reading"] for r in matches}) == 1:
                p["nameReading"] = matches[0]["reading"]
                applied.append({"id": p["id"], "sources": matches})
            elif matches:
                review.append({"id": p["id"], "sources": matches, "reason": "近接する同名の自然地名で読みが異なる"})
    return {"applied": applied, "review": review}


def main():
    # 生成の入口はベクトル優先の処理に統一する。
    from build_vector import main as build
    build()


if __name__ == "__main__":
    main()
