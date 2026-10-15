"""保存済み PBF の名前付きノードを、山以外も含む地点 JSON にする。"""

import argparse
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from all_points_io import read_points, write_points, write_report, input_record

if __package__:
    from scripts.build_data import parse_elevation, read_aliases, wikipedia_url, wikidata_url, verified_source_url, normalize_timestamp
else:
    from build_data import parse_elevation, read_aliases, wikipedia_url, wikidata_url, verified_source_url, normalize_timestamp


CLASSIFICATION_KEYS = ("natural", "place", "amenity", "tourism", "historic", "railway", "highway",
                       "waterway", "water", "landuse", "leisure", "shop", "office", "man_made",
                       "aeroway", "building", "boundary", "power", "sport", "emergency", "geological",
                       "healthcare", "military", "public_transport")


def node_point(identifier, latitude, longitude, tags):
    name = tags.get("name:ja", "").strip() or tags.get("name", "").strip()
    if not name:
        return None
    elevation = parse_elevation(tags.get("ele"))
    classification = "; ".join(key + "=" + tags[key].strip() for key in CLASSIFICATION_KEYS if tags.get(key, "").strip())
    info = {"dataSource": "osm"}
    if classification:
        info["classification"] = classification
    if elevation is not None:
        info["elevationSource"] = "osm"
    return {"id": f"osm:{identifier}",
            "type": "peak" if tags.get("natural") in ("peak", "volcano") else None,
            "name": name, "latitude": latitude, "longitude": longitude,
            "elevationM": elevation, "nameReading": tags.get("name:ja-Hira", "").strip() or None,
            "aliases": read_aliases(tags, name), "wikipediaUrl": wikipedia_url(tags.get("wikipedia")),
            "wikidataUrl": wikidata_url(tags.get("wikidata")), "provenance": info}


def build(pbf, output, peaks=None):
    import osmium

    pbf = Path(pbf)
    source_url = verified_source_url(pbf)
    metadata = {}
    overrides = {}
    if peaks:
        for point in read_points(peaks, metadata=metadata):
            if point["id"] in overrides:
                raise ValueError("山頂版の ID が重複しています")
            overrides[point["id"]] = point
    remaining = set(overrides)
    with osmium.io.Reader(str(pbf), osmium.osm.NODE) as reader:
        timestamp = normalize_timestamp(reader.header().get("osmosis_replication_timestamp"))

    def points():
        count = 0
        # 座標を持つノードだけを使い、線や面の代表点は作らない。
        for node in osmium.FileProcessor(str(pbf), entities=osmium.osm.NODE).with_filter(osmium.filter.KeyFilter("name", "name:ja")):
            if not node.location.valid():
                continue
            point = node_point(node.id, node.location.lat, node.location.lon, dict(node.tags))
            if point is None:
                continue
            saved = overrides.get(point["id"])
            if saved is not None:
                if point["type"] != "peak" or any(abs(saved[key] - point[key]) > 1e-7 for key in ("latitude", "longitude")):
                    raise ValueError("山頂版と PBF の地点が一致しません")
                point = saved
                remaining.discard(point["id"])
            yield point
            count += 1
            if count % 100000 == 0:
                print(f"抽出: {count:,} 地点", flush=True)
        if remaining:
            raise ValueError("山頂版にだけ存在する地点があります。元データの版を確認してください")

    from scripts.point_document import source_entries
    report = write_points(points(), output, "osm-all-points", tags=metadata.get("tags"), metadata={"sources":source_entries("osm-peaks",source_url,"ODbL-1.0","© OpenStreetMap contributors",metadata.get("sources"))})
    report.update(sourceTimestamp=timestamp, sourceUrl=source_url, inputs=[input_record(pbf)],
                  scope="名前付きノードのみ。線・面・relation の代表点は作らない。",
                  license="ODbL-1.0", attribution="© OpenStreetMap contributors")
    if peaks:
        report["inputs"].append(input_record(peaks))
    write_report(output, report)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pbf", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--peaks", type=Path, help="同じ PBF から生成した山頂版。補足済みの地点を引き継ぐ")
    args = parser.parse_args()
    report = build(args.pbf, args.output, args.peaks)
    print(json.dumps({key: report[key] for key in ("pointCount", "peakCount", "elevationCount")}, ensure_ascii=False))


if __name__ == "__main__":
    main()
