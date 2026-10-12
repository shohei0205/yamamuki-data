"""全国の自然地名タイルを全走査し、取得結果を SQLite に保存して山 JSON を作る。"""

import argparse
from collections import OrderedDict
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
import gzip
import hashlib
import http.client
import json
import math
from pathlib import Path
import sqlite3
import threading
import time

from download import Cache
from generate import CSV_URL, DEM_URL, NATURAL_URL, merge, read_csv, read_natural, tile_position, write

NATIONAL_BBOX = (122, 20, 154, 46)
LOCAL = threading.local()


def now():
    return datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def bounds(bbox):
    west, south, east, north = bbox
    if not (west < east and south < north):
        raise ValueError("範囲は西・南・東・北の順に指定してください")
    x1, y1 = tile_position(north, west, 15)
    x2, y2 = tile_position(south, east, 15)
    return math.floor(x1), math.floor(y1), math.floor(x2), math.floor(y2)


def tile_at(index, area):
    x1, y1, x2, y2 = area
    height = y2 - y1 + 1
    if not 0 <= index < (x2 - x1 + 1) * height:
        raise ValueError("タイル番号が範囲外です")
    return x1 + index // height, y1 + index % height


def open_database(path, bbox):
    database = sqlite3.connect(path)
    database.execute("PRAGMA journal_mode=WAL")
    database.execute("CREATE TABLE IF NOT EXISTS config (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
    database.execute("""CREATE TABLE IF NOT EXISTS tiles (
        number INTEGER PRIMARY KEY, status INTEGER NOT NULL CHECK(status IN (200,404)),
        body BLOB, metadata NOT NULL)""")
    database.execute("CREATE TABLE IF NOT EXISTS missing_metadata (id INTEGER PRIMARY KEY, metadata TEXT UNIQUE NOT NULL)")
    expected = json.dumps({"bbox": list(bbox), "zoom": 15, "urlTemplate": NATURAL_URL}, sort_keys=True)
    row = database.execute("SELECT value FROM config WHERE key='scan'").fetchone()
    if row and row[0] != expected:
        database.close()
        raise ValueError("保存済みの全取得範囲と今回の指定が異なります")
    database.execute("INSERT OR IGNORE INTO config VALUES ('scan', ?)", (expected,))
    database.commit()
    return database


def store_tile(database, result, area, references):
    """404 の URL を番号から復元し、共通の取得記録を一度だけ保存する。"""
    number, status, body, metadata = result
    if status == 404:
        details = json.loads(metadata)
        x, y = tile_at(number, area)
        if details.get("url") == NATURAL_URL.format(x=x, y=y):
            del details["url"]
            template = json.dumps(details, ensure_ascii=False, sort_keys=True)
            if template not in references:
                database.execute("INSERT OR IGNORE INTO missing_metadata(metadata) VALUES (?)", (template,))
                references[template] = database.execute(
                    "SELECT id FROM missing_metadata WHERE metadata=?", (template,)).fetchone()[0]
            metadata = references[template]
    database.execute("INSERT INTO tiles VALUES (?,?,?,?)", (number, status, body, metadata))


def restore_metadata(database, number, status, metadata, area):
    # 旧形式は JSON の文字列、新形式の 404 は共通記録の番号。
    if status == 404 and (isinstance(metadata, int) or metadata.isdigit()):
        row = database.execute("SELECT metadata FROM missing_metadata WHERE id=?", (metadata,)).fetchone()
        details = json.loads(row[0])
        x, y = tile_at(number, area)
        details["url"] = NATURAL_URL.format(x=x, y=y)
        return details
    return json.loads(metadata)


def missing_numbers(database, start, stop):
    done = {row[0] for row in database.execute(
        "SELECT number FROM tiles WHERE number>=? AND number<?", (start, stop))}
    return [number for number in range(start, stop) if number not in done]


def connection():
    if not getattr(LOCAL, "connection", None):
        LOCAL.connection = http.client.HTTPSConnection("cyberjapandata.gsi.go.jp", timeout=30)
    return LOCAL.connection


def disconnect():
    if getattr(LOCAL, "connection", None):
        LOCAL.connection.close()
        LOCAL.connection = None


def fetch(number, area):
    x, y = tile_at(number, area)
    url = NATURAL_URL.format(x=x, y=y)
    path = f"/xyz/experimental_nnfpt/15/{x}/{y}.geojson"
    attempt = 0
    while True:
        try:
            client = connection()
            client.request("GET", path, headers={"User-Agent": "yamamuki-data-gsi-prototype/1", "Accept-Encoding": "identity"})
            response = client.getresponse()
            status = response.status
            payload = response.read(8 * 1024 * 1024 + 1)
            if len(payload) > 8 * 1024 * 1024:
                raise ValueError(f"タイルの応答が上限を超えました: {url}")
            if status in (200, 404):
                metadata = {"url": url, "status": status, "fetchedAt": now(),
                            "lastModified": response.getheader("Last-Modified"), "etag": response.getheader("ETag"),
                            "sha256": hashlib.sha256(payload).hexdigest() if status == 200 else None,
                            "sizeBytes": len(payload) if status == 200 else 0}
                if status == 200:
                    data = json.loads(payload)
                    if data.get("type") != "FeatureCollection" or not isinstance(data.get("features"), list):
                        raise ValueError(f"自然地名タイルの形式が不正です: {url}")
                return number, status, gzip.compress(payload, mtime=0) if status == 200 else None, json.dumps(metadata, ensure_ascii=False)
            if status not in (429, 500, 502, 503, 504):
                raise ValueError(f"自然地名タイルの HTTP {status}: {url}")
            retry_after = response.getheader("Retry-After")
            delay = max(min(300, int(retry_after)) if retry_after and retry_after.isdigit() else 0,
                        min(60, 2 ** min(attempt, 6)))
        except (OSError, http.client.HTTPException) as error:
            delay = min(60, 2 ** min(attempt, 6))
            if attempt % 6 == 0:
                print(f"通信を再試行: {url}: {error}", flush=True)
        disconnect()
        time.sleep(delay)
        attempt += 1


def status_file(output, **state):
    path = output / "progress.json"
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps({"updatedAt": now(), **state}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    # Windows で閲覧中の進捗ファイルが一時的にロックされても取得を止めない。
    for attempt in range(8):
        try:
            temporary.replace(path)
            break
        except PermissionError:
            if attempt == 7:
                raise
            time.sleep(min(1, 0.1 * 2 ** attempt))


def scan(database, area, output, workers):
    total = (area[2] - area[0] + 1) * (area[3] - area[1] + 1)
    completed, positive = database.execute("SELECT count(*),coalesce(sum(status=200),0) FROM tiles").fetchone()
    references = {}
    started_count, started = completed, time.monotonic()
    last = started
    print(f"全国取得を開始: {completed}/{total} タイル、同時接続 {workers}", flush=True)
    def publish():
        elapsed = time.monotonic() - started
        speed = (completed - started_count) / elapsed if elapsed else 0
        state = {"phase": "fetching", "completedTiles": completed, "totalTiles": total,
                 "existingTiles": positive, "missingTiles": completed - positive,
                 "tilesPerSecond": round(speed, 2), "bbox": list(NATIONAL_BBOX)}
        status_file(output, **state)
        print(json.dumps(state, ensure_ascii=False), flush=True)
    publish()
    with ThreadPoolExecutor(max_workers=workers) as executor:
        for start in range(0, total, 1000):
            pending = missing_numbers(database, start, min(start + 1000, total))
            futures = [executor.submit(fetch, number, area) for number in pending]
            for future in as_completed(futures):
                result = future.result()
                store_tile(database, result, area, references)
                completed += 1
                positive += result[1] == 200
                if time.monotonic() - last >= 10:
                    database.commit()
                    publish()
                    last = time.monotonic()
            database.commit()
    publish()
    database.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    return total, positive


def finish(database, database_path, area, output, cache_path, version, total, positive, offline=False):
    stored, first, last = database.execute("SELECT count(*),min(number),max(number) FROM tiles").fetchone()
    if stored != total or first != 0 or last != total - 1:
        raise ValueError("全タイルの取得が完了していません")
    cache = Cache(cache_path, offline=offline)
    csv_points = read_csv(cache.get(CSV_URL))
    natural, skipped, sources, reading_variants = [], [], [], []
    for number, compressed, metadata_json in database.execute("SELECT number,body,metadata FROM tiles WHERE status=200 ORDER BY number"):
        payload = gzip.decompress(compressed)
        metadata = json.loads(metadata_json)
        if hashlib.sha256(payload).hexdigest() != metadata["sha256"]:
            raise ValueError("保存済み自然地名タイルのハッシュが一致しません")
        for feature in json.loads(payload)["features"]:
            properties = feature["properties"]
            kana = properties.get("kana", "")
            if properties.get("type") == "山" and "," in kana:
                reading_variants.append({"name": properties.get("name"), "readings": kana.split(","), "sourceUrl": metadata["url"]})
        points, omissions = read_natural(payload)
        natural.extend(points)
        skipped.extend({**item, "sourceUrl": metadata["url"]} for item in omissions)
        sources.append(metadata)
    points, conflicts = merge(csv_points, natural)
    # 同じ DEM タイルを解析し直さず、地表標高を補う。
    dem_rows, dem_count = OrderedDict(), 0
    status_file(output, phase="elevation", completedTiles=total, totalTiles=total, pointCount=len(points))
    for i, p in enumerate(points, 1):
        if p["elevationM"] is None:
            x, y = tile_position(p["latitude"], p["longitude"], 14)
            tile = int(x), int(y)
            if tile not in dem_rows:
                payload = cache.get(DEM_URL.format(x=tile[0], y=tile[1]), missing_ok=True)
                rows = None if payload is None else [line.split(",") for line in payload.decode("ascii").splitlines()]
                if rows is not None and (len(rows) != 256 or any(len(row) != 256 for row in rows)):
                    raise ValueError("標高タイルの形式が不正です")
                dem_rows[tile] = rows
                if len(dem_rows) > 32:
                    dem_rows.popitem(last=False)
            rows = dem_rows[tile]
            dem_rows.move_to_end(tile)
            if rows is not None:
                value = rows[min(255, int((y % 1) * 256))][min(255, int((x % 1) * 256))]
                if value != "e":
                    elevation = float(value)
                    if not math.isfinite(elevation):
                        raise ValueError("標高が不正です")
                    p["elevationM"] = elevation
                    dem_count += 1
        if i % 100 == 0:
            print(f"標高補完: {i}/{len(points)} 地点", flush=True)
            status_file(output, phase="elevation", completedTiles=total, totalTiles=total, completedPoints=i, pointCount=len(points))
    database.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    with database_path.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    report = {"bbox": list(NATIONAL_BBOX), "pointCount": len(points), "mountainListCount": len(csv_points),
              "naturalRepresentativePointCount": sum(p["id"].startswith("gsi-natural:") for p in points),
              "readingCount": sum(bool(p["nameReading"]) for p in points),
              "elevationCount": sum(p["elevationM"] is not None for p in points), "demElevationCount": dem_count,
              "conflicts": conflicts, "skipped": skipped, "readingVariants": reading_variants, "sources": sources + list(cache.used.values()),
              "scan": {"complete": True, "zoom": 15, "tileBounds": list(area), "totalTiles": total,
                       "existingTiles": positive, "missingTiles": total - positive,
                       "databaseFile": str(database_path.resolve()), "databaseSha256": digest,
                       "missingTileDetails": "404 を含む全応答の取得日時は SQLite に保存（404 の共通記録は missing_metadata）"}}
    manifest = write(points, report, output, version)
    status_file(output, phase="complete", completedTiles=total, totalTiles=total, pointCount=len(points),
                readingCount=report["readingCount"], elevationCount=report["elevationCount"], sizeBytes=manifest["sizeBytes"])
    print("全国の取得・JSON 生成が完了しました", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("build/gsi-national-all"))
    parser.add_argument("--cache", type=Path, default=Path("build/gsi-cache"))
    parser.add_argument("--regenerate-only", action="store_true", help="保存した元データだけで再生成し、通信しない")
    parser.add_argument("--workers", type=int, default=8)
    parser.add_argument("--version", default="local-national-all-20261012")
    args = parser.parse_args()
    if not 1 <= args.workers <= 16:
        parser.error("同時接続数は 1～16 にしてください")
    args.output.mkdir(parents=True, exist_ok=True)
    database_path = args.output / "natural-tiles.sqlite3"
    database = open_database(database_path, NATIONAL_BBOX)
    area = bounds(NATIONAL_BBOX)
    try:
        if args.regenerate_only:
            total = (area[2] - area[0] + 1) * (area[3] - area[1] + 1)
            positive = database.execute("SELECT count(*) FROM tiles WHERE status=200").fetchone()[0]
        else:
            total, positive = scan(database, area, args.output, args.workers)
        finish(database, database_path, area, args.output, args.cache, args.version, total, positive, offline=args.regenerate_only)
    except BaseException as error:
        database.commit()
        status_file(args.output, phase="failed", error=str(error))
        raise
    finally:
        database.close()


if __name__ == "__main__":
    main()
