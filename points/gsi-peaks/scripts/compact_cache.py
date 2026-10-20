"""自然地名の保存済み SQLite を、全記録を照合して小さい形式へ変換する。"""

import argparse
import json
from pathlib import Path
import sqlite3

from fetch_all import bounds, open_database, restore_metadata, store_tile


def compact(source, output):
    if output.exists() or source.resolve() == output.resolve():
        raise ValueError("出力先には存在しない別のファイルを指定してください")
    original = sqlite3.connect(source.resolve().as_uri() + "?mode=ro", uri=True)
    converted = None
    try:
        config = dict(original.execute("SELECT key,value FROM config"))
        bbox = json.loads(config["scan"])["bbox"]
        area = bounds(bbox)
        converted = open_database(output, bbox)
        converted.executemany("INSERT OR REPLACE INTO config VALUES (?,?)", config.items())
        references = {}
        for count, (number, status, body, metadata) in enumerate(original.execute(
                "SELECT number,status,body,metadata FROM tiles ORDER BY number"), 1):
            details = restore_metadata(original, number, status, metadata, area)
            store_tile(converted, (number, status, body, json.dumps(details, ensure_ascii=False)), area, references)
            if count % 100000 == 0:
                converted.commit()
                print(f"変換: {count:,} 区画", flush=True)
        converted.commit()
        # 全 GeoJSON の圧縮バイト列と、URL・日時を含む全取得記録を照合する。
        old_rows = original.execute("SELECT number,status,body,metadata FROM tiles ORDER BY number")
        new_rows = converted.execute("SELECT number,status,body,metadata FROM tiles ORDER BY number")
        count = 0
        for old, new in zip(old_rows, new_rows, strict=True):
            if old[:3] != new[:3] or restore_metadata(original, old[0], old[1], old[3], area) != restore_metadata(
                    converted, new[0], new[1], new[3], area):
                raise ValueError(f"変換後の取得記録が一致しません: {old[0]}")
            count += 1
        if converted.execute("PRAGMA integrity_check").fetchone()[0] != "ok":
            raise ValueError("変換後の SQLite が破損しています")
        converted.execute("PRAGMA wal_checkpoint(TRUNCATE)")
        print(f"全 {count:,} 区画の元データと取得記録の一致を確認しました", flush=True)
    finally:
        original.close()
        if converted is not None:
            converted.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    compact(args.source, args.output)


if __name__ == "__main__":
    main()
