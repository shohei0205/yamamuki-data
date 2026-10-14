"""GSI 山頂と、収録済み山頂を除いた全国の名前付き注記を統合する。"""

import argparse
from contextlib import closing
from copy import deepcopy
import json
from pathlib import Path
import sqlite3
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from all_points_io import read_points, write_points, write_report, input_record


def excluded_annotations(peaks, comparison):
    identifiers = {p["id"].removeprefix("gsi-vector:") for p in peaks if p["id"].startswith("gsi-vector:")}
    for row in comparison["enrichment"]:
        if row.get("source") == "mountainList":
            identifiers.add(row["vectorId"].removeprefix("gsi-vector:"))
    # 代表注記から複製への対応を、記録の並び順に依存せず展開する。
    pending = list(comparison["duplicates"])
    while pending:
        remaining = []
        for row in pending:
            if row["representativeId"].removeprefix("gsi-vector:") in identifiers:
                identifiers.add(row["vectorId"].removeprefix("gsi-vector:"))
            else:
                remaining.append(row)
        if len(remaining) == len(pending):
            break
        pending = remaining
    coordinates = {(p["name"], round(p["latitude"], 7), round(p["longitude"], 7)) for p in peaks}
    return identifiers, coordinates


def annotation_points(database, peaks, comparison):
    identifiers, coordinates = excluded_annotations(peaks, comparison)
    database = Path(database)
    if not database.is_file():
        raise FileNotFoundError(f"注記の抽出 DB がありません: {database}")
    with closing(sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True)) as db:
        complete = db.execute("SELECT value FROM state WHERE key='complete'").fetchone()
        if not complete or complete[0] != "true":
            raise ValueError("注記の抽出が完了していません。先に extract_annotations.py を実行してください")
        for identifier, body, code in db.execute("SELECT id,body,code FROM points ORDER BY id"):
            point = json.loads(body)
            if identifier.removeprefix("gsi-vector:") in identifiers or (point["name"], round(point["latitude"], 7), round(point["longitude"], 7)) in coordinates:
                continue
            point["id"] = "gsi-vector:" + identifier.removeprefix("gsi-vector:")
            point["type"] = None
            point["provenance"] = {"dataSource": "vector"}
            if code is not None:
                point["provenance"]["classification"] = str(code)
            yield point


def preserve_height(point, previous):
    """同じ名前・座標の注記にだけ、取得済み標高を引き継ぐ。"""
    saved = previous.get(point["id"])
    if point.get("elevationM") is None and saved and all(point.get(key) == saved.get(key) for key in ("name", "latitude", "longitude")):
        point = deepcopy(point)
        point["elevationM"] = saved["elevationM"]
        source = saved.get("provenance", {}).get("elevationSource")
        if source is not None:
            point.setdefault("provenance", {})["elevationSource"] = source
    return point


def build(database, peaks_path, comparison_path, output, previous_path=None):
    inputs = [input_record(path) for path in (peaks_path, comparison_path, database)]
    if previous_path:
        inputs.append(input_record(previous_path))
    metadata = {}
    peaks = list(read_points(peaks_path, metadata=metadata))
    comparison = json.loads(Path(comparison_path).read_text(encoding="utf-8"))
    previous = {}
    if previous_path:
        for point in read_points(previous_path):
            if point.get("type") != "peak" and point.get("elevationM") is not None:
                previous[point["id"]] = point

    def points():
        yield from peaks
        for point in annotation_points(database, peaks, comparison):
            yield preserve_height(point, previous)

    report = write_points(points(), output, "gsi-all-points", tags=metadata.get("tags"), metadata={k:v for k,v in metadata.items() if k != "tags"})
    report.update(inputs=inputs,
                  scope="山頂版と名称付き単一点注記。数字だけの注記・空名称・タイル外の複製・収録済み山頂注記を除外。",
                  elevationPolicy="標高と読みを推測で補わない。注記の標高は山頂の標高とは限らない。",
                  license="PDL-1.0", attribution="国土地理院")
    write_report(output, report)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", required=True, type=Path)
    parser.add_argument("--peaks", type=Path, default=Path(__file__).with_name("gsi-peaks.json"))
    parser.add_argument("--comparison", required=True, type=Path, help="山頂版と同時に生成した vector-comparison.json")
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--previous", type=Path, help="同じ名前・座標の注記に取得済み標高を引き継ぐ全部入り JSON")
    args = parser.parse_args()
    report = build(args.database, args.peaks, args.comparison, args.output, args.previous)
    print(json.dumps({key: report[key] for key in ("pointCount", "peakCount", "elevationCount")}, ensure_ascii=False))


if __name__ == "__main__":
    main()
