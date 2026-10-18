"""全国を含む範囲の標高 PNG を保存し、中断した取得を再開する。"""

import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import hashlib
import json
import math
from pathlib import Path
import sqlite3
import shutil
import threading
import time
import urllib.error
import urllib.request

from ridges import SOURCE, decode_png, tile_position


def bounds(bbox, zoom):
    west, south, east, north = bbox
    if not (-180 < west < east < 180 and -85 < south < north < 85):
        raise ValueError("取得範囲が不正です")
    left, top = tile_position(west, north, zoom)
    right, bottom = tile_position(east, south, zoom)
    return math.floor(left), math.floor(top), math.ceil(right)-1, math.ceil(bottom)-1


# 陸地の多い地域から先に確認する。範囲外も後で走査して離島を落とさない。
PRIORITY_AREAS = [
    ("中部", [137,35,139,37]),
    ("本州中央", [135,34,141,38]),
    ("東北", [139,38,142.2,41.6]),
    ("北海道", [139.5,41.4,145.9,45.6]),
    ("中国・四国・近畿", [131,32.5,136,35.8]),
    ("九州", [129,30,132.2,34.8]),
    ("南西諸島", [122.8,24,131.5,30]),
    ("伊豆・小笠原諸島", [139,24,143,35]),
]


def tile_batches(area, zoom):
    x0,y0,x1,y1=area
    width=x1-x0+1
    seen=bytearray(width*(y1-y0+1))
    regions=[(name,bounds(bbox,zoom)) for name,bbox in PRIORITY_AREAS]+[("残りの範囲",area)]
    for name,(left,top,right,bottom) in regions:
        batch=[]
        for y in range(max(y0,top),min(y1,bottom)+1):
            for x in range(max(x0,left),min(x1,right)+1):
                index=(y-y0)*width+x-x0
                if seen[index]:
                    continue
                seen[index]=1
                batch.append((x,y))
                if len(batch)==32:
                    yield name,batch
                    batch=[]
        if batch:
            yield name,batch


class RateLimit:
    def __init__(self, rate):
        self.interval = 1 / rate
        self.next = 0
        self.lock = threading.Lock()

    def wait(self):
        with self.lock:
            now = time.monotonic()
            delay = max(0, self.next-now)
            self.next = max(now, self.next)+self.interval
        time.sleep(delay)


def fetch_tile(x, y, zoom, cache, limit):
    path = cache / str(zoom) / str(x) / f"{y}.png"
    url = f"{SOURCE}/{zoom}/{x}/{y}.png"
    cached = path.exists()
    for attempt in range(8):
        try:
            if cached:
                body = path.read_bytes()
            else:
                limit.wait()
                request = urllib.request.Request(url, headers={"User-Agent": "yamamuki-terrain-prototype/1"})
                with urllib.request.urlopen(request, timeout=45) as response:
                    body = response.read()
            decode_png(body)
            if not cached:
                path.parent.mkdir(parents=True, exist_ok=True)
                temporary = path.with_suffix(".part")
                temporary.write_bytes(body)
                temporary.replace(path)
            return x, y, 200, len(body), hashlib.sha256(body).hexdigest(), datetime.now(timezone.utc).isoformat(), url
        except urllib.error.HTTPError as error:
            if error.code == 404:
                return x, y, 404, 0, None, datetime.now(timezone.utc).isoformat(), url
            if error.code not in (429, 500, 502, 503, 504) or attempt == 7:
                raise
            retry = error.headers.get("Retry-After", "") if error.headers else ""
            time.sleep(min(300, max(2**attempt, int(retry) if retry.isdigit() else 0)))
        except (urllib.error.URLError, TimeoutError):
            if attempt == 7:
                raise
            time.sleep(min(120, 2**attempt))


def run(args):
    area = bounds(args.bbox, args.zoom)
    args.output.mkdir(parents=True, exist_ok=True)
    args.cache.mkdir(parents=True, exist_ok=True)
    # 同じ保存先で二重起動しない。終了時に OS がロックを解放する。
    lock = sqlite3.connect(args.output / "run-lock.sqlite3", timeout=0)
    lock.execute("CREATE TABLE IF NOT EXISTS run_lock (id INTEGER)")
    lock.commit()
    lock.execute("BEGIN EXCLUSIVE")
    db = sqlite3.connect(args.output / "tiles.sqlite3")
    db.execute("PRAGMA journal_mode=WAL")
    db.execute("CREATE TABLE IF NOT EXISTS config (value TEXT NOT NULL)")
    db.execute("CREATE TABLE IF NOT EXISTS tiles (x INTEGER, y INTEGER, status INTEGER, bytes INTEGER, sha256 TEXT, fetched_at TEXT, url TEXT, PRIMARY KEY(x,y))")
    config = json.dumps({"bbox": args.bbox, "zoom": args.zoom, "source": SOURCE, "cache": str(args.cache.resolve())}, sort_keys=True)
    old = db.execute("SELECT value FROM config").fetchone()
    if old and old[0] != config:
        raise ValueError("保存済みの取得条件と一致しません。別の output を指定してください")
    if not old:
        db.execute("INSERT INTO config VALUES (?)", (config,))
    db.commit()
    x0, y0, x1, y1 = area
    total = (x1-x0+1)*(y1-y0+1)
    done, found, size = db.execute("SELECT count(*),coalesce(sum(status=200),0),coalesce(sum(bytes),0) FROM tiles").fetchone()
    current_region = "準備中"
    def progress(phase, error=None):
        record = {"phase": phase, "updatedAt": datetime.now(timezone.utc).isoformat(), "totalTiles": total,
                  "currentRegion": current_region, "checkedTiles": done, "completedTiles": done, "savedTiles": found, "missingTiles": done-found, "pngBytes": size,
                  "bbox": args.bbox, "zoom": args.zoom, "cache": str(args.cache.resolve()), "error": error}
        path = args.output / "progress.json"
        temporary = path.with_suffix(".tmp")
        temporary.write_text(json.dumps(record, ensure_ascii=False, indent=2), encoding="utf-8", newline="\n")
        # Windows のファイル監視が一時的に進捗ファイルを開く場合は待つ。
        for attempt in range(10):
            try:
                temporary.replace(path)
                break
            except PermissionError:
                if attempt == 9:
                    raise
                time.sleep(0.1)
    limit = RateLimit(args.rate)
    progress("fetching")
    try:
        with ThreadPoolExecutor(max_workers=args.workers) as pool:
            for current_region, batch in tile_batches(area, args.zoom):
                if (args.output / "STOP").exists():
                    progress("stopped")
                    return
                pending=[]
                for x,y in batch:
                    stored=db.execute("SELECT status FROM tiles WHERE x=? AND y=?",(x,y)).fetchone()
                    if stored is None or (stored[0]==200 and not (args.cache/str(args.zoom)/str(x)/f"{y}.png").exists()):
                        pending.append((x,y))
                if not pending:
                    continue
                if min(shutil.disk_usage(args.cache).free, shutil.disk_usage(args.output).free) < 10_000_000_000:
                    progress("disk_space_low", "空き容量が 10 GB 未満のため停止しました")
                    return
                futures = [pool.submit(fetch_tile, x, y, args.zoom, args.cache, limit) for x,y in pending]
                for future in futures:
                    row = future.result()
                    previous = db.execute("SELECT status,bytes FROM tiles WHERE x=? AND y=?", row[:2]).fetchone()
                    done += previous is None
                    found += (row[2] == 200) - (previous is not None and previous[0] == 200)
                    size += row[3] - (previous[1] if previous else 0)
                    db.execute("INSERT OR REPLACE INTO tiles VALUES (?,?,?,?,?,?,?)", row)
                db.commit()
                progress("fetching")
        progress("complete")
    except BaseException as error:
        db.commit()
        progress("failed", str(error))
        raise
    finally:
        db.close()
        lock.close()


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--bbox", nargs=4, type=float, default=[122., 20., 154., 46.])
    result.add_argument("--zoom", type=int, default=14, choices=range(1,15))
    result.add_argument("--cache", type=Path, default=Path("build/terrain-cache"))
    result.add_argument("--output", type=Path, default=Path("build/terrain-national"))
    result.add_argument("--workers", type=int, default=4, choices=range(1,9))
    result.add_argument("--rate", type=float, default=8)
    return result


if __name__ == "__main__":
    args = parser().parse_args()
    if not 0 < args.rate <= 20:
        raise ValueError("毎秒の問い合わせ数は 0 より大きく 20 以下にしてください")
    run(args)
