"""Rebuild a shared terrain SQLite physical layout without changing logical data."""
import argparse
from contextlib import closing
import itertools
import json
import sqlite3
import time
from pathlib import Path

SCHEMA = [
    ("metadata", "CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT) WITHOUT ROWID", "key"),
    ("tiles", "CREATE TABLE tiles(z INTEGER NOT NULL,x INTEGER NOT NULL,y INTEGER NOT NULL,origin_x INTEGER,origin_y INTEGER,kind TEXT,points INTEGER,xyz_sha256 TEXT,payload BLOB,depth INTEGER,PRIMARY KEY(z,x,y))", "z,x,y"),
    ("parents", "CREATE TABLE parents(z INTEGER,x INTEGER,y INTEGER,slot INTEGER,parent_z INTEGER,parent_x INTEGER,parent_y INTEGER,PRIMARY KEY(z,x,y,slot)) WITHOUT ROWID", "z,x,y,slot"),
]

def statistics(connection, path):
    return {
        "file": str(path),
        "bytes": path.stat().st_size,
        "page_size": connection.execute("PRAGMA page_size").fetchone()[0],
        "free_pages": connection.execute("PRAGMA freelist_count").fetchone()[0],
        "tiles": connection.execute("SELECT count(*) FROM tiles").fetchone()[0],
        "blob_bytes": connection.execute("SELECT sum(length(payload)) FROM tiles").fetchone()[0],
    }

def compact(source, destination, page_size=512):
    source, destination = Path(source).resolve(), Path(destination).resolve()
    temporary = destination.with_name(destination.name + ".creating")
    report_path = destination.with_name(destination.name + ".layout.json")
    if not source.is_file():
        raise FileNotFoundError(source)
    if source == destination or destination.exists() or temporary.exists() or report_path.exists():
        raise FileExistsError("Existing source/output/report must never be overwritten")
    destination.parent.mkdir(parents=True, exist_ok=True)
    start = time.monotonic()
    with closing(sqlite3.connect(source.as_uri() + "?mode=ro", uri=True)) as original:
        expected = {row[0] for row in original.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")}
        if expected != {row[0] for row in SCHEMA}:
            raise ValueError("Unsupported shared terrain database schema")
        if original.execute("SELECT value FROM metadata WHERE key='complete'").fetchone() != ("true",):
            raise ValueError("Only completed databases can be compacted")
        before = statistics(original, source)
        with closing(sqlite3.connect(temporary.as_uri() + "?mode=rwc", uri=True)) as target:
            target.execute("PRAGMA page_size=" + str(page_size))
            target.execute("PRAGMA journal_mode=DELETE")
            target.execute("PRAGMA synchronous=FULL")
            target.execute("ATTACH DATABASE ? AS source", (source.as_uri() + "?mode=ro",))
            target.execute("BEGIN IMMEDIATE")
            for table, ddl, order in SCHEMA:
                target.execute(ddl)
                target.execute("INSERT INTO main." + table + " SELECT * FROM source." + table + " ORDER BY " + order)
            target.commit()
            if target.execute("PRAGMA integrity_check").fetchall() != [("ok",)]:
                raise ValueError("SQLite integrity check failed")
            counts = {}
            missing = object()
            for table, _, order in SCHEMA:
                count = 0
                sql = "SELECT * FROM " + table + " ORDER BY " + order
                for old, new in itertools.zip_longest(original.execute(sql), target.execute(sql), fillvalue=missing):
                    if old != new:
                        raise ValueError("Logical row mismatch in " + table + " at " + str(count))
                    count += 1
                counts[table] = count
            # Check every parent edge in the copied database.
            broken = target.execute("""
                SELECT count(*) FROM parents p
                LEFT JOIN tiles child ON child.z=p.z AND child.x=p.x AND child.y=p.y
                LEFT JOIN tiles parent ON parent.z=p.parent_z AND parent.x=p.parent_x AND parent.y=p.parent_y
                WHERE child.z IS NULL OR parent.z IS NULL
            """).fetchone()[0]
            if broken:
                raise ValueError("Broken parent references: " + str(broken))
            after = statistics(target, temporary)
    # Windows rename refuses to overwrite an existing destination.
    temporary.rename(destination)
    after["file"] = str(destination)
    report = {"before": before, "after": after, "verified_rows": counts,
              "all_columns_and_blobs_equal": True, "parent_references_valid": True,
              "saved_bytes": before["bytes"]-after["bytes"], "seconds": time.monotonic()-start}
    report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return report

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--page-size", type=int, default=512, choices=[512,1024,2048,4096,8192])
    args = parser.parse_args()
    print(json.dumps(compact(args.input, args.output, args.page_size), ensure_ascii=False, indent=2))
