"""ベクトルタイルを山の一覧の第一ソースとし、他の国土地理院データで補完する。"""

import argparse
from collections import Counter, defaultdict
from contextlib import closing
import hashlib
import json
from pathlib import Path

from download import Cache
from generate import CSV_URL, distance, point, read_csv, reading, write
from heights import extract_heights, apply_annotations, fill_dem
from natural_sources import acquire, load, target_tiles, open_cache
from vector import extract, normalized, PMTILES_URL


def name_index(points):
    index = defaultdict(dict)
    for p in points:
        for name in {p["name"], *p["aliases"]}:
            index[normalized(name)][p["id"]] = p
    return index


def nearby(p, index, radius=100):
    matches = {}
    for name in {p["name"], *p["aliases"]}:
        for identifier, source in index.get(normalized(name), {}).items():
            d = distance(p, source)
            if d <= radius:
                matches[identifier] = (d, source)
    return sorted(matches.values(), key=lambda v: (v[0], v[1]["id"]))


def primary_points(records, mountains, natural, features, reading_radius=100):
    # ズーム 16 用・17 用の注記で数 m だけ座標が違うコピーを除く。
    vectors, duplicates = [], []
    vector_index = defaultdict(dict)
    for record in sorted(records, key=lambda r: (r["properties"].get("vt_code", 810), r["point"]["id"])):
        p = record["point"]
        matches = nearby(p, vector_index, 10)
        if len(matches) == 1:
            duplicates.append({"vectorId": p["id"], "representativeId": matches[0][1]["id"], "distanceM": round(matches[0][0], 2)})
            continue
        vectors.append(dict(p, aliases=list(p["aliases"])))
        vector_index[normalized(p["name"])][p["id"]] = vectors[-1]
    mountain_index, natural_index = name_index(mountains), name_index(natural)
    anchor_index = name_index([dict(p, aliases=list(p["aliases"])) for p in vectors])
    reading_index = defaultdict(list)
    vector_names = {normalized(p["name"]) for p in vectors}
    invalid_readings = []
    for feature in features:
        properties = feature["properties"]
        for name_key, kana_key in (("name", "kana"), ("Aname", "Akana")):
            name, kana = properties.get(name_key, ""), properties.get(kana_key, "")
            if name and kana and normalized(name) in vector_names:
                try:
                    values = [reading(value) for value in kana.split(",")]
                except ValueError:
                    invalid_readings.append({"name": name, "reading": kana, "sourceUrl": feature["sourceUrl"], "reason": "自然地名の読みをかなに正規化できない"})
                    continue
                reading_index[normalized(name)].append({**feature, "sourceName": name, "reading": values[0], "readingVariants": values})
    # 山岳一覧と注記が一対一になる場合だけ、山頂座標・索引 ID を優先する。
    # 山頂と山名注記は数百 m 離れることがある。索引を持つ山岳一覧との一対一照合は 1 km まで調べる。
    mountain_matches = {p["id"]: nearby(p, mountain_index, 1000) for p in vectors}
    reverse = Counter(source["id"] for matches in mountain_matches.values() for _, source in matches if len(matches) == 1)
    links, reviews, readings = [], invalid_readings, []
    for p in vectors:
        anchor = p.copy()
        matches = mountain_matches[p["id"]]
        mountain = matches[0][1] if len(matches) == 1 and reverse[matches[0][1]["id"]] == 1 else None
        if matches and mountain is None:
            reviews.append({"id": p["id"], "reason": "山岳一覧と山名注記の対応が一対一ではない", "candidateIds": [source["id"] for _, source in matches]})
        if mountain:
            p.update(id=mountain["id"], latitude=mountain["latitude"], longitude=mountain["longitude"], elevationM=mountain["elevationM"])
            p["aliases"] = list(dict.fromkeys([*mountain["aliases"], mountain["name"]]))
            if normalized(p["name"]) == normalized(mountain["name"]):
                p["nameReading"] = mountain["nameReading"]
            links.append({"vectorId": anchor["id"], "id": p["id"], "source": "mountainList", "distanceM": round(matches[0][0], 2)})
        natural_matches = nearby(anchor, natural_index)
        if len(natural_matches) == 1:
            source = natural_matches[0][1]
            p["aliases"] = list(dict.fromkeys([*p["aliases"], *source["aliases"], source["name"]]))
            links.append({"vectorId": anchor["id"], "id": p["id"], "source": "natural", "sourceId": source["id"], "distanceM": round(natural_matches[0][0], 2)})
        elif natural_matches:
            reviews.append({"id": p["id"], "reason": "近くに同名・別名の自然地名が複数ある", "candidateIds": [source["id"] for _, source in natural_matches]})
        candidates = [r for r in reading_index.get(normalized(anchor["name"]), []) if distance(anchor, r) <= 100]
        # 注記位置が移動した場合の読みだけを、相互に一意な同名地点から補う。
        if not candidates and p["nameReading"] is None and reading_radius > 100:
            far = [r for r in reading_index.get(normalized(anchor["name"]), []) if distance(anchor, r) <= reading_radius]
            unique = {(r["latitude"], r["longitude"], r["reading"]): r for r in far}
            if len(unique) == 1:
                source = next(iter(unique.values()))
                source_point = point("reading-source", source["sourceName"], source["reading"], source["latitude"], source["longitude"])
                reverse_matches = nearby(source_point, anchor_index, reading_radius)
                if len(reverse_matches) == 1 and reverse_matches[0][1]["id"] == anchor["id"]:
                    candidates = [source]
        values = {r["reading"] for r in candidates}
        if len(values) == 1:
            value = next(iter(values))
            if p["nameReading"] is None:
                p["nameReading"] = value
                readings.append({"id": p["id"], "reading": value, "sources": [{"url": r["sourceUrl"], "name": r["sourceName"], "distanceM": round(distance(anchor, r), 2), "naturalType": r["properties"]["type"], "readingVariants": r["readingVariants"]} for r in candidates]})
            elif p["nameReading"] != value:
                reviews.append({"id": p["id"], "reason": "山岳一覧と自然地名の読みが異なるため山岳一覧を優先", "mountainListReading": p["nameReading"], "naturalReading": value})
        elif values:
            reviews.append({"id": p["id"], "reason": "近くにある同名の自然地名で読みが異なる", "readings": sorted(values)})
        p["aliases"] = [name for name in p["aliases"] if name != p["name"]]
    # 第一ソースにない山だけを最後に追加し、位置の移動と思われるものは保留する。
    result = list(vectors)
    index = name_index(result)
    identifiers = {p["id"] for p in result}
    fallback, matched = [], []
    for source in [*mountains, *natural]:
        if source["id"] in identifiers:
            continue
        matches = nearby(source, index, 1000)
        if len(matches) == 1 and matches[0][0] <= 100:
            matched.append({"sourceId": source["id"], "id": matches[0][1]["id"], "distanceM": round(matches[0][0], 2)})
        elif matches:
            reviews.append({"sourceId": source["id"], "name": source["name"], "reason": "同名・別名の地点が近くにあるため追加を保留", "candidates": [{"id": a["id"], "distanceM": round(d, 2)} for d, a in matches]})
        else:
            p = dict(source, aliases=list(source["aliases"]))
            result.append(p)
            identifiers.add(p["id"])
            fallback.append(p["id"])
            for name in {p["name"], *p["aliases"]}:
                index[normalized(name)][p["id"]] = p
    return sorted(result, key=lambda p: p["id"]), {"vectorPointCount": len(vectors), "duplicates": duplicates,
            "enrichment": links, "naturalReadings": readings, "fallbackIds": fallback, "matchedFallback": matched, "review": reviews}


def elevations(points, cache):
    return fill_dem(points, cache)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--cache", type=Path, default=Path("build/gsi-cache"))
    parser.add_argument("--output", type=Path, default=Path("build/gsi-national-primary"))
    parser.add_argument("--extraction-cache", type=Path, help="抽出済みの vector-annotations.sqlite3 を再利用する")
    parser.add_argument("--natural-database", type=Path, help="取得済み全国自然地名 DB。存在確認済みタイルを全件再利用する")
    parser.add_argument("--max-natural-tiles", type=int, default=100000)
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--version", default="local-primary-20261013")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    # 元データを検証してから抽出キャッシュを使う。
    archive_info = json.loads((args.archive.parent / "optimal_bvmap-source.json").read_bytes())
    with args.archive.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    if archive_info["url"] != PMTILES_URL or archive_info["sha256"] != digest or archive_info["sizeBytes"] != args.archive.stat().st_size:
        raise ValueError("PMTiles と保存記録が一致しません")
    records, skipped, processed = extract(args.archive, args.extraction_cache or args.output / "vector-annotations.sqlite3")
    cache = Cache(args.cache, offline=args.offline)
    mountains = read_csv(cache.get(CSV_URL))
    selected, scan = acquire([r["point"] for r in records] + mountains, args.output / "natural-selected.sqlite3",
                             args.natural_database, args.offline, args.max_natural_tiles)
    natural, features, omissions, natural_sources = load(args.output / "natural-selected.sqlite3", selected)
    points, comparison = primary_points(records, mountains, natural, features)
    pending = [p for p in points if p["nameReading"] is None]
    # 完全な旧 DB があれば全存在タイルを既に読んでいるため、広い探索のための通信は不要。
    if pending and not scan["includesCompletedScan"]:
        if len(selected | target_tiles(pending, 1000)) > args.max_natural_tiles:
            raise ValueError("自然地名の選択取得の合計が上限を超えました")
        extra, extra_scan = acquire(pending, args.output / "natural-selected.sqlite3", args.natural_database,
                                    args.offline, args.max_natural_tiles, radius=1000)
        selected |= extra
        scan["networkRequests"] += extra_scan["networkRequests"]
        scan["network404Count"] += extra_scan["network404Count"]
        scan["targetTileCount"] = len(target_tiles([r["point"] for r in records] + mountains) | target_tiles(pending, 1000))
        scan["selectedTileCount"] = len(selected)
        scan["reusedTiles"] = len(selected) - scan["networkRequests"]
        with closing(open_cache(args.output / "natural-selected.sqlite3")) as db:
            scan["existingTiles"] = sum(db.execute("SELECT status FROM responses WHERE x=? AND y=?", tile).fetchone()[0] == 200 for tile in selected)
        scan["missingTiles"] = len(selected) - scan["existingTiles"]
        natural, features, omissions, natural_sources = load(args.output / "natural-selected.sqlite3", selected)
    points, comparison = primary_points(records, mountains, natural, features, reading_radius=1000)
    scan["readingSearchRadiusM"] = 1000
    measurements = extract_heights(args.archive, args.output / "height-annotations.sqlite3", points)
    annotation_report = apply_annotations(points, measurements)
    dem_report = elevations(points, cache)
    codes = {record["point"]["id"]: str(record["properties"]["vt_code"]) for record in records}
    mountain_codes = {row["id"]: codes[row["vectorId"]] for row in comparison["enrichment"] if row["source"] == "mountainList"}
    height_sources = {row["id"]: row["source"] for row in [*annotation_report["applied"], *dem_report["assignments"]]}
    for p in points:
        source = "mountainList" if p["id"].startswith("gsi-mountain-list:") else "natural" if p["id"].startswith("gsi-natural:") else "vector"
        p["provenance"] = {"dataSource": source,
                           "classification": codes.get(p["id"], mountain_codes.get(p["id"], "山（自然地名）" if source == "natural" else "peak"))}
        elevation_source = "mountainList" if source == "mountainList" else height_sources.get(p["id"])
        if elevation_source is not None:
            p["provenance"]["elevationSource"] = elevation_source
    report = {"pointCount": len(points), "primarySource": "optimal_bvmap-v1", "vectorAnnotationCount": len(records),
              "vectorPointCount": comparison["vectorPointCount"], "fallbackCount": len(comparison["fallbackIds"]),
              "readingCount": sum(bool(p["nameReading"]) for p in points), "elevationCount": sum(p["elevationM"] is not None for p in points),
              "demElevationCount": dem_report["pointCount"], "demSources": dem_report["bySource"],
              "annotationElevationCount": len(annotation_report["applied"]), "processedVectorTiles": processed, "naturalAcquisition": scan,
              "reviewCount": len(comparison["review"]), "skipped": skipped + omissions,
              "sources": sorted([archive_info, *natural_sources, *cache.used.values()], key=lambda s: s["url"])}
    write(points, report, args.output, args.version)
    path = args.output / "manifest.json"
    manifest = json.loads(path.read_bytes())
    manifest.update(attribution="出典：国土地理院（最適化ベクトルタイル・日本の主な山岳標高一覧・自然地名・標高タイル）を加工して作成",
                    sourceUrl=PMTILES_URL, sourceDate=None,
                    elevationPolicy="山岳一覧の公称標高、25 m 以内で双方に一意な三角点・標高点、DEM1A・DEM5A・DEM5B・DEM5C・DEM10B の順に採用。DEM は指定座標の地表標高であり、最高地点の標高とは限らない。",
                    coordinatePolicy="山名・座標は最適化ベクトルタイルを優先し、一意に照合できた山岳一覧の最高地点を優先する。未収録の山は山岳一覧・取得済み自然地名から追加する。注記・代表点は最高地点とは限らない。")
    path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    elevation_sources = ([{"id": p["id"], "source": "mountainList", "elevationM": p["elevationM"], "sourceUrl": CSV_URL}
                          for p in points if p["id"].startswith("gsi-mountain-list:")]
                         + annotation_report["applied"] + dem_report["assignments"])
    (args.output / "elevation-sources.json").write_text(json.dumps({"points": sorted(elevation_sources, key=lambda r:r["id"]),
        "annotationUnresolved": annotation_report["unresolved"], "missingIds": dem_report["missingIds"]}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    for name, data in (("vector-comparison.json", comparison), ("vector-only-points.json", [r["point"] for r in records])):
        (args.output / name).write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({k: v for k, v in report.items() if k not in ("sources", "skipped")}, ensure_ascii=False), flush=True)


if __name__ == "__main__":
    main()
