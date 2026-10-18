from datetime import datetime, timezone
"""国土地理院の公式 CSV・自然地名・標高タイルだけから、山頂データを生成する。"""

import argparse
import csv
import gzip
import hashlib
import io
import json
import math
from pathlib import Path
import re
import sys
import unicodedata

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from document_metadata import point_document_json, distributed_point, metadata_arrays, validate_metadata, ordered_point, source_entries

from download import Cache

CSV_URL = "https://www.gsi.go.jp/KOKUJYOHO/MOUNTAIN/1003zan20260331.csv"
# 公式 CSV に実際の「?」が含まれる表記を、同じ山の公式ベクトルタイルで確認した名前に補正する。
# 索引番号と補正前の表記が両方一致する場合だけ適用し、元 CSV は変更しない。
MOUNTAIN_NAME_REPAIRS = {
    "199": ("薬?山", "薬萊山"),
    "204-3": ("?風岳", "屛風岳"),
    "226": ("?差岳", "朳差岳"),
    "662": ("?風山", "屛風山"),
}
NATURAL_URL = "https://cyberjapandata.gsi.go.jp/xyz/experimental_nnfpt/15/{x}/{y}.geojson"
DEM_URL = "https://cyberjapandata.gsi.go.jp/xyz/dem/14/{x}/{y}.txt"
LICENSE = "PDL-1.0 https://www.gsi.go.jp/kikakuchousei/kikakuchousei40182.html"
ATTRIBUTION = "出典：国土地理院（日本の主な山岳標高一覧・自然地名・標高タイル）を加工して作成"


def reading(value):
    value = unicodedata.normalize("NFKC", value.strip())
    if not value or not re.fullmatch(r"[ぁ-ゖァ-ヶー]+", value):
        raise ValueError(f"かなとして解釈できない読みです: {value}")
    return "".join(chr(ord(c) - 96) if "ァ" <= c <= "ヶ" else c for c in value)


def name_parts(name, kana):
    # ＜ ＞内は山頂名。総称の読みを山頂の読みに流用しない。
    if ("＜" in name) != ("＜" in kana):
        raise ValueError("山頂名と読みの括弧が対応していません")
    def select(value):
        value = value.strip()
        if "＜" in value:
            match = re.fullmatch(r"[^＜]+＜([^＜＞]+)＞", value)
            if not match:
                raise ValueError("山頂名の括弧を解釈できません")
            value = match[1]
        value = value.removeprefix("[").removesuffix("]").removeprefix("［").removesuffix("］")
        match = re.fullmatch(r"([^（）]+)(?:（([^（）]+)）)?", value)
        if not match:
            raise ValueError("山名の括弧を解釈できません")
        return match[1].strip(), match[2].strip() if match[2] else None
    main, alias = select(name)
    kana_main, kana_alias = select(kana)
    if bool(alias) != bool(kana_alias):
        raise ValueError("山名と読みの別称が対応していません")
    return main, reading(kana_main), [alias] if alias and alias != main else []


def coordinate(lat, lon):
    lat, lon = float(lat), float(lon)
    if not (-85.05112878 <= lat <= 85.05112878 and -180 <= lon < 180):
        raise ValueError("座標が不正です")
    return lat, lon


def point(identifier, name, kana, lat, lon, elevation=None, aliases=None):
    lat, lon = coordinate(lat, lon)
    if not name.strip() or (elevation is not None and not math.isfinite(elevation)):
        raise ValueError("山名または標高が不正です")
    return {"id": identifier, "type": "peak", "name": name.strip(), "nameReading": kana,
            "latitude": lat, "longitude": lon, "elevationM": elevation,
            "aliases": list(dict.fromkeys(a for a in (aliases or []) if a and a != name)),
            "wikipediaUrl": None, "wikidataUrl": None}


def read_csv(payload):
    rows = csv.DictReader(io.StringIO(payload.decode("cp932")))
    required = {"索引番号", "山名＜山頂名＞", "山名よみ＜山頂名よみ＞", "標高値(m)", "緯度", "経度"}
    if not required <= set(rows.fieldnames or []):
        raise ValueError("主な山岳一覧の CSV 列が変わっています")
    points, seen = [], set()
    for row in rows:
        index = row["索引番号"]
        if not re.fullmatch(r"[1-9][0-9]*(?:-[1-9][0-9]*)?", index) or index in seen:
            raise ValueError("山岳一覧の索引番号が不正または重複しています")
        seen.add(index)
        name, kana, aliases = name_parts(row["山名＜山頂名＞"], row["山名よみ＜山頂名よみ＞"])
        repair = MOUNTAIN_NAME_REPAIRS.get(index)
        if repair and name == repair[0]:
            name = repair[1]
        points.append(point("gsi-mountain-list:" + index, name, kana, row["緯度"], row["経度"], float(row["標高値(m)"]), aliases))
    if not points:
        raise ValueError("山岳一覧が空です")
    return points


def tile_position(lat, lon, zoom):
    lat, lon = coordinate(lat, lon)
    n = 2 ** zoom
    return (lon + 180) / 360 * n, (1 - math.asinh(math.tan(math.radians(lat))) / math.pi) / 2 * n


def tiles_for_bbox(bbox, limit=10000):
    west, south, east, north = bbox
    if not west < east or not south < north:
        raise ValueError("範囲は西・南・東・北の順に指定してください")
    x1, y1 = tile_position(north, west, 15)
    x2, y2 = tile_position(south, east, 15)
    bounds = (math.floor(x1), math.floor(y1), math.floor(x2), math.floor(y2))
    count = (bounds[2] - bounds[0] + 1) * (bounds[3] - bounds[1] + 1)
    if count > limit:
        raise ValueError(f"取得対象 {count} タイルが上限 {limit} を超えます。範囲を狭めてください")
    return [(x, y) for x in range(bounds[0], bounds[2] + 1) for y in range(bounds[1], bounds[3] + 1)]


def inside(p, bbox):
    return bbox is None or bbox[0] <= p["longitude"] <= bbox[2] and bbox[1] <= p["latitude"] <= bbox[3]


def read_natural(payload):
    data = json.loads(payload)
    if data.get("type") != "FeatureCollection" or not isinstance(data.get("features"), list):
        raise ValueError("自然地名タイルの形式が変わっています")
    points, skipped = [], []
    for feature in data["features"]:
        properties, geometry = feature["properties"], feature["geometry"]
        if properties.get("type") != "山":
            continue
        if geometry["type"] != "Point" or len(geometry["coordinates"]) != 2:
            raise ValueError("山の自然地名が点ではありません")
        if properties.get("gaijiFlg", "0") != "0":
            skipped.append({"name": properties.get("name"), "reason": "外字の復元が必要"})
            continue
        lon, lat = geometry["coordinates"]
        name, kana = properties["name"].strip(), properties.get("kana", "").strip()
        identifier = "gsi-natural:" + hashlib.sha256(f"{name}|{float(lat):.7f}|{float(lon):.7f}".encode()).hexdigest()[:24]
        aliases = [properties.get("Aname", "").strip()]
        # 複数の読みは元資料の先頭を表示用に使う。全候補は元タイルに保持する。
        kana_values = [reading(value) for value in kana.split(",")] if kana else []
        points.append(point(identifier, name, kana_values[0] if kana_values else None, lat, lon, aliases=aliases))
    return points, skipped


def distance(a, b):
    lat1, lat2 = math.radians(a["latitude"]), math.radians(b["latitude"])
    h = math.sin((lat2 - lat1) / 2) ** 2 + math.cos(lat1) * math.cos(lat2) * math.sin(math.radians(b["longitude"] - a["longitude"]) / 2) ** 2
    return 12742000 * math.asin(min(1, math.sqrt(h)))


def merge(csv_points, natural_points):
    result = {p["id"]: p.copy() for p in csv_points}
    conflicts = []
    for p in natural_points:
        if p["id"] in result:
            if result[p["id"]] != p:
                raise ValueError("同一 ID の自然地名の内容が異なります")
            continue
        matches = [a for a in csv_points if a["name"] == p["name"] and distance(a, p) <= 100]
        if len(matches) == 1:
            a = result[matches[0]["id"]]
            if a["nameReading"] != p["nameReading"]:
                conflicts.append({"id": a["id"], "name": a["name"], "mountainListReading": a["nameReading"], "naturalReading": p["nameReading"]})
            a["aliases"] = list(dict.fromkeys(a["aliases"] + p["aliases"]))
        elif matches:
            conflicts.append({"id": p["id"], "name": p["name"], "reason": "近くに同名の山岳一覧の地点が複数ある"})
        else:
            result[p["id"]] = p.copy()
    return sorted(result.values(), key=lambda p: p["id"]), conflicts


def dem_elevation(payload, lat, lon):
    rows = payload.decode("ascii").splitlines()
    if len(rows) != 256 or any(len(row.split(",")) != 256 for row in rows):
        raise ValueError("標高タイルは 256 × 256 である必要があります")
    x, y = tile_position(lat, lon, 14)
    value = rows[min(255, int((y % 1) * 256))].split(",")[min(255, int((x % 1) * 256))]
    if value == "e":
        return None
    elevation = float(value)
    if not math.isfinite(elevation):
        raise ValueError("標高タイルの値が不正です")
    return elevation


def generate(cache, bbox=None, natural=False, dem=False, max_tiles=10000, csv_url=CSV_URL):
    csv_points = [p for p in read_csv(cache.get(csv_url)) if inside(p, bbox)]
    natural_points, skipped = [], []
    if natural:
        if bbox is None:
            raise ValueError("自然地名の取得には --bbox が必要です")
        tiles = tiles_for_bbox(bbox, max_tiles)
        print(f"自然地名を取得: {len(tiles)} タイル", flush=True)
        # キャッシュのメタデータを一つのスレッドから更新し、進捗を表示する。
        for i, (x, y) in enumerate(tiles, 1):
            payload = cache.get(NATURAL_URL.format(x=x, y=y), missing_ok=True)
            if payload is not None:
                points, omissions = read_natural(payload)
                natural_points.extend(p for p in points if inside(p, bbox))
                skipped.extend(omissions)
            if i % 25 == 0 or i == len(tiles):
                print(f"自然地名: {i}/{len(tiles)} タイル", flush=True)
    points, conflicts = merge(csv_points, natural_points)
    dem_count = 0
    if dem:
        for p in points:
            if p["elevationM"] is not None:
                continue
            x, y = tile_position(p["latitude"], p["longitude"], 14)
            payload = cache.get(DEM_URL.format(x=int(x), y=int(y)), missing_ok=True)
            if payload is not None:
                p["elevationM"] = dem_elevation(payload, p["latitude"], p["longitude"])
                dem_count += p["elevationM"] is not None
    if not points:
        raise ValueError("指定範囲に生成できる山がありません")
    return points, {"bbox": bbox, "pointCount": len(points), "mountainListCount": len(csv_points),
                    "naturalRepresentativePointCount": sum(p["id"].startswith("gsi-natural:") for p in points),
                    "readingCount": sum(bool(p["nameReading"]) for p in points),
                    "elevationCount": sum(p["elevationM"] is not None for p in points),
                    "demElevationCount": dem_count, "conflicts": conflicts, "skipped": skipped,
                    "sources": sorted(cache.used.values(), key=lambda value: value["url"])}


def write(points, report, output, version, tags=None, metadata=None):
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", version):
        raise ValueError("版の文字が不正です")
    legacy = dict(metadata or {})
    source_url = legacy.pop("sourceUrl",CSV_URL)
    license = legacy.pop("license",LICENSE)
    attribution = legacy.pop("attribution",ATTRIBUTION)
    sources = source_entries("gsi-peaks",source_url,license,attribution,legacy.pop("supplementSources",None))
    # 実際に使った国土地理院の資料を、取得記録の種別ごとに一覧へ残す。
    for item in report.get("sources",[]):
        url=item["url"]
        if "/experimental_nnfpt/" in url:
            sources=source_entries("gsi-natural","https://cyberjapandata.gsi.go.jp/xyz/experimental_nnfpt/{z}/{x}/{y}.geojson",license,attribution,sources)
        elif "/xyz/dem" in url:
            sources=source_entries("gsi-dem","https://maps.gsi.go.jp/development/ichiran.html",license,attribution,sources)
        elif url==CSV_URL and source_url!=CSV_URL:
            sources=source_entries("gsi-mountain-list",CSV_URL,license,attribution,sources)
    info = {"schemaVersion":6,"pointCount":len(points),"sources":sources,**legacy,"tags":{} if tags is None else tags}
    validate_metadata(info, len(points))
    raw = point_document_json({"schemaFormat":"yamamuki-points","schemaVersion":6,"metadata":metadata_arrays({k:v for k,v in info.items() if k != "schemaVersion"}),"points":[distributed_point(point) for point in points]}).encode()
    stream = io.BytesIO()
    with gzip.GzipFile(filename="", mode="wb", fileobj=stream, mtime=0) as compressed:
        compressed.write(raw)
    payload = stream.getvalue()
    if len(payload) > 5000000:
        raise ValueError("圧縮後のサイズが 5 MB を超えました")
    output = Path(output)
    output.mkdir(parents=True, exist_ok=True)
    manifest = {"schemaVersion": 6, "dataSchemaVersion": 6, "name": "山頂（国土地理院）",
                "version": version, "fileName": "gsi-peaks.json.gz", "pointCount": len(points),
                "downloadUrl": f"https://github.com/shohei0205/yamamuki-data/releases/download/gsi-peaks-{version}/gsi-peaks.json.gz",
                "sha256": hashlib.sha256(payload).hexdigest(), "sizeBytes": len(payload), "uncompressedSizeBytes": len(raw),
                "sources":sources,
                "sourceDate": "2026-03-31", "experimental": True,
                "coordinatePolicy": "主な山岳一覧の最高地点。自然地名を追加した場合は山の代表点を含む。",
                "elevationPolicy": "山岳一覧の標高を優先。任意の DEM 補完は代表点の地表標高であり公称の山頂標高ではない。",
                "sourceFiles": report["sources"]}
    manifest.update({k:v for k,v in legacy.items() if k != "schemaVersion"})
    if tags:
        manifest["tags"]=tags
    manifest["dataTimestamp"] = datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")
    from document_metadata import copy_manifest_metadata
    copy_manifest_metadata(manifest, {k:v for k,v in info.items() if k != "schemaVersion"})
    (output / "gsi-peaks.json.gz").write_bytes(payload)
    (output / "gsi-peaks.json").write_bytes(raw)
    for name, data in (("manifest.json", manifest), ("report.json", report)):
        (output / name).write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache", type=Path, default=Path("build/gsi-cache"))
    parser.add_argument("--output", type=Path, default=Path("build/gsi-peaks"))
    parser.add_argument("--version", default="local-20260331")
    parser.add_argument("--bbox", type=float, nargs=4, metavar=("WEST", "SOUTH", "EAST", "NORTH"))
    parser.add_argument("--natural", action="store_true")
    parser.add_argument("--dem", action="store_true")
    parser.add_argument("--max-tiles", type=int, default=10000)
    parser.add_argument("--refresh", action="store_true")
    args = parser.parse_args()
    # 通信前に入力を検査する。
    if args.natural and args.bbox is None:
        parser.error("--natural には --bbox が必要です")
    if args.bbox:
        tiles_for_bbox(args.bbox, args.max_tiles if args.natural else 2 ** 32)
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", args.version):
        parser.error("版の文字が不正です")
    cache = Cache(args.cache, args.refresh)
    points, report = generate(cache, args.bbox, args.natural, args.dem, args.max_tiles)
    manifest = write(points, report, args.output, args.version)
    print(json.dumps({key: value for key, value in report.items() if key not in ("sources", "conflicts", "skipped")}, ensure_ascii=False))
    print(f"保存先: {args.output}（圧縮 {manifest['sizeBytes']} バイト）")


if __name__ == "__main__":
    main()
