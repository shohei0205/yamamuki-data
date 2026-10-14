"""山に関係する注記分類の地点に、注記座標の DEM 標高を補う。"""

import argparse
import json
from pathlib import Path
import sys
import time
from urllib.error import HTTPError

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from all_points_io import read_points, write_points, write_report, input_record
from download import Cache
from heights import fill_dem

RELATED_CODES = {"311", "312", "313", "314", "315", "316", "323", "331", "332", "333", "810", "830", "831", "832"}


class RetryCache(Cache):
    def get(self, url, missing_ok=False):
        for attempt in range(4):
            try:
                return super().get(url, missing_ok=missing_ok)
            except HTTPError as error:
                if error.code not in (429, 500, 502, 503, 504) or attempt == 3:
                    raise
                delay = (10, 30, 60)[attempt]
                print(f"一時エラー {error.code}: {delay} 秒後に再試行します", flush=True)
                time.sleep(delay)


def selected_annotations(points, codes=RELATED_CODES):
    return [p for p in points if p.get("type") != "peak" and p.get("elevationM") is None
            and p.get("provenance", {}).get("classification") in codes]


def build(source, output, cache_directory, offline=False, workers=2, codes=RELATED_CODES):
    source, output = Path(source), Path(output)
    if source.resolve() in {(output / name).resolve() for name in ("gsi-all-points.json", "gsi-all-points.json.gz")}:
        raise ValueError("入力を保持するため、標高補完の出力先は別フォルダーを指定してください")
    selected = selected_annotations(read_points(source), codes)
    print(f"標高取得の対象: {len(selected):,} 地点", flush=True)
    output.mkdir(parents=True, exist_ok=True)
    cache = RetryCache(cache_directory, offline=offline)
    try:
        dem = fill_dem(selected, cache, workers=workers)
    finally:
        (output / "related-dem-source-records.json").write_text(json.dumps(sorted(cache.used.values(), key=lambda r: r["url"]), ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    assignments = {row["id"]: row for row in dem["assignments"]}
    for point in selected:
        if point["id"] in assignments:
            point["provenance"]["elevationSource"] = assignments[point["id"]]["source"]
    by_id = {p["id"]: p for p in selected}
    report = write_points((by_id.get(p["id"], p) for p in read_points(source)), output, "gsi-all-points")
    report.update(inputs=[input_record(source)], license="PDL-1.0", attribution="国土地理院",
                  relatedAnnotationElevations={"selectedCodes": sorted(codes, key=int), "targetCount": len(selected),
                    "acquiredCount": dem["pointCount"], "missingCount": len(dem["missingIds"]), "bySource": dem["bySource"],
                    "coordinatePolicy": "注記座標を維持した DEM の地表標高。山頂・最高地点の標高とは限らない。"})
    write_report(output, report)
    (output / "related-elevation-assignments.json").write_text(json.dumps(dem, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    if selected:
        write_points(selected, output, "mountain-related-annotation-points")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--cache", required=True, type=Path)
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--workers", type=int, choices=range(1, 5), default=2)
    parser.add_argument("--codes", nargs="+", default=sorted(RELATED_CODES), choices=sorted(RELATED_CODES))
    args = parser.parse_args()
    report = build(args.input, args.output, args.cache, args.offline, args.workers, set(args.codes))
    print(json.dumps(report["relatedAnnotationElevations"], ensure_ascii=False))


if __name__ == "__main__":
    main()
