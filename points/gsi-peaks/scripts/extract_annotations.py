"""保存済み PMTiles の名前付き単一点注記を、再開可能な SQLite に抽出する。"""

import argparse
from collections import Counter
from contextlib import closing
import gzip
import hashlib
import json
import math
import mmap
from pathlib import Path
import re
import sqlite3
import time


def annotations(payload, z, x, y):
    from mapbox_vector_tile.Mapbox.vector_tile_pb2 import tile
    message = tile()
    message.ParseFromString(gzip.decompress(payload) if payload[:2] == b"\x1f\x8b" else payload)
    skipped = Counter()
    records = []
    for layer in message.layers:
        if layer.name != "Anno":
            continue
        for feature in layer.features:
            tags = list(feature.tags)
            props = {layer.keys[tags[i]]: layer.values[tags[i+1]].ListFields()[0][1] for i in range(0, len(tags), 2)}
            name = str(props.get("vt_text", "")).strip()
            if not name:
                skipped["emptyName"] += 1
                continue
            if re.fullmatch(r"[+-]?[0-9]+(?:\.[0-9]+)?", name):
                skipped["numericOnly"] += 1
                continue
            geometry = list(feature.geometry)
            if feature.type != 1 or len(geometry) != 3 or geometry[0] != 9:
                skipped["notSinglePoint"] += 1
                continue
            a, b = [(v >> 1) ^ -(v & 1) for v in geometry[1:]]
            if not (0 <= a < layer.extent and 0 <= b < layer.extent):
                skipped["tileBuffer"] += 1
                continue
            lon = (x + a / layer.extent) / 2**z * 360 - 180
            lat = math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * (y + b / layer.extent) / 2**z))))
            identifier = hashlib.sha256(f"{name}|{lat:.7f}|{lon:.7f}".encode()).hexdigest()[:24]
            point = {"id": identifier, "type": None, "name": name, "nameReading": None,
                     "latitude": lat, "longitude": lon, "elevationM": None,
                     "aliases": [], "wikipediaUrl": None, "wikidataUrl": None}
            records.append((identifier, json.dumps(point, ensure_ascii=False, separators=(",", ":")),
                            props.get("vt_code"), json.dumps({"tile": [z, x, y], "properties": props}, ensure_ascii=False, separators=(",", ":"))))
    return records, skipped


def extract(archive, database):
    from pmtiles.reader import Reader
    from pmtiles.tile import deserialize_directory, tileid_to_zxy
    archive, database = Path(archive).resolve(), Path(database)
    database.parent.mkdir(parents=True, exist_ok=True)
    with closing(sqlite3.connect(database)) as db:
        db.execute("CREATE TABLE IF NOT EXISTS state(key TEXT PRIMARY KEY,value TEXT)")
        db.execute("CREATE TABLE IF NOT EXISTS points(id TEXT PRIMARY KEY,body TEXT,code INTEGER,source TEXT)")
        identity = json.dumps([str(archive), archive.stat().st_size, archive.stat().st_mtime_ns])
        saved = db.execute("SELECT value FROM state WHERE key='archive'").fetchone()
        if saved and saved[0] != identity:
            raise ValueError("抽出元が異なります。別の抽出 DB を指定してください")
        db.execute("INSERT OR IGNORE INTO state VALUES('archive',?)", (identity,))
        last = int((db.execute("SELECT value FROM state WHERE key='last'").fetchone() or ["-1"])[0])
        identifier = last
        processed = int((db.execute("SELECT value FROM state WHERE key='processed'").fetchone() or ["0"])[0])
        skipped = Counter(json.loads((db.execute("SELECT value FROM state WHERE key='skipped'").fetchone() or ["{}"])[0]))

        def checkpoint(complete=False):
            rows = [('last', str(identifier)), ('processed', str(processed)), ('skipped', json.dumps(skipped))]
            if complete:
                rows.append(('complete', 'true'))
            db.executemany("INSERT OR REPLACE INTO state VALUES(?,?)", rows)
            db.commit()

        with archive.open("rb") as stream, mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ) as mapping:
            header = Reader(lambda o, length: mapping[o:o+length]).header()
            if header['max_zoom'] != 16 or archive.stat().st_size != header['tile_data_offset'] + header['tile_data_length']:
                raise ValueError("PMTiles が未完了、または想定と異なるズームです")

            def entries(offset, length):
                for entry in deserialize_directory(mapping[offset:offset+length]):
                    if entry.run_length:
                        yield entry
                    else:
                        yield from entries(header['leaf_directory_offset'] + entry.offset, entry.length)

            tick = time.monotonic()
            for entry in entries(header['root_offset'], header['root_length']):
                if entry.tile_id + entry.run_length - 1 <= last:
                    continue
                for identifier in range(max(last + 1, entry.tile_id), entry.tile_id + entry.run_length):
                    z, x, y = tileid_to_zxy(identifier)
                    if z != 16:
                        continue
                    offset = header['tile_data_offset'] + entry.offset
                    records, rejected = annotations(mapping[offset:offset+entry.length], z, x, y)
                    db.executemany("INSERT OR IGNORE INTO points VALUES(?,?,?,?)", records)
                    skipped.update(rejected)
                    processed += 1
                    if processed % 1000 == 0:
                        checkpoint()
                    if time.monotonic() - tick >= 20:
                        print(f"抽出: {processed:,} タイル", flush=True)
                        tick = time.monotonic()
            checkpoint(complete=True)
        return {"pointCount": db.execute("SELECT count(*) FROM points").fetchone()[0],
                "processedTiles": processed, "skipped": dict(skipped)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--database", required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps(extract(args.archive, args.database), ensure_ascii=False))


if __name__ == "__main__":
    main()
