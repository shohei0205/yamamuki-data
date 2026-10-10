"""osmium で抽出した山頂ノードを、全国版の配布ファイルにする。"""

import argparse
import os
from datetime import datetime, timezone
import gzip
import hashlib
import json
import logging
import math
from pathlib import Path
import re
import subprocess
import tempfile
import time
from urllib.parse import quote
import xml.etree.ElementTree as ET


if __package__:
    from scripts.release_channels import download_url
    from scripts.graphics import validate_graphics
    from scripts.point_tags import validate_tags
    from scripts.tag_json import apply_tag_jsons
    from scripts.supplements import DEFAULT_SUPPLEMENTS, read_supplements, apply_supplements, write_companions
else:
    from release_channels import download_url
    from graphics import validate_graphics
    from point_tags import validate_tags
    from tag_json import apply_tag_jsons
    from supplements import DEFAULT_SUPPLEMENTS, read_supplements, apply_supplements, write_companions


SOURCE_URL = "https://download.geofabrik.de/asia/japan-latest.osm.pbf"
FILE_NAME = "osm-peaks.json.gz"
MAX_SIZE_BYTES = 5_000_000


def read_aliases(tags, name):
    """日本語別名を先に並べ、空欄・表示名と同じ名前・重複を除く。"""
    aliases = []
    for key in ("alt_name:ja", "alt_name"):
        for value in tags.get(key, "").split(";"):
            value = value.strip()
            if value and value != name and value not in aliases:
                aliases.append(value)
    return aliases


def wikipedia_url(raw):
    """OSM の「言語:記事名」を、記事内の節にも対応した HTTPS URL にする。"""
    language, separator, title = (raw or "").strip().partition(":")
    if not separator or not re.fullmatch(r"[a-z]{2,3}(?:-[a-z0-9]+)*", language):
        return None
    page, fragment_separator, fragment = title.strip().partition("#")
    if not page.strip():
        return None
    url = f"https://{language}.wikipedia.org/wiki/{quote(page.strip().replace(' ', '_'), safe='')}"
    if fragment_separator and fragment:
        url += "#" + quote(fragment.replace(" ", "_"), safe="")
    return url


def wikidata_url(raw):
    identifier = (raw or "").strip()
    return f"https://www.wikidata.org/wiki/{identifier}" if re.fullmatch(r"Q[1-9][0-9]*", identifier) else None


def parse_elevation(raw):
    """アプリと同様に、標高の単位と区切りを読み取る。"""
    first = (raw or "").split(";")[0].strip().lower()
    match = re.fullmatch(r"(-?[\d,]*\.?\d+)\s*(m|meters?|metres?|ft|feet|')?", first)
    if not match:
        return None
    value = float(match[1].replace(",", ""))
    if match[2] in ("ft", "feet", "'"):
        value *= 0.3048
    return value if math.isfinite(value) else None


def read_mountains(path):
    logging.info("抽出したノードを読み取っています: %s", path)
    mountains = {}
    latest_timestamp = None
    context = ET.iterparse(path, events=("start", "end"))
    _, root = next(context)
    if root.tag != "osm":
        raise ValueError("OSM XML ではありません")
    for event, node in context:
        if event != "end" or node.tag not in ("node", "way", "relation"):
            continue
        if node.tag == "node":
            tags = {tag.attrib["k"]: tag.attrib["v"] for tag in node.findall("tag")}
            name = tags.get("name:ja", "").strip() or tags.get("name", "").strip()
            if tags.get("natural") in ("peak", "volcano") and name:
                osm_id = int(node.attrib["id"])
                lat, lon = float(node.attrib["lat"]), float(node.attrib["lon"])
                if osm_id <= 0 or not (-90 <= lat <= 90 and -180 <= lon <= 180):
                    raise ValueError(f"ノード {osm_id} の ID または座標が不正です")
                if osm_id in mountains:
                    raise ValueError(f"ノード {osm_id} が重複しています")
                timestamp = node.attrib.get("timestamp")
                if not timestamp:
                    raise ValueError(f"ノード {osm_id} の最終編集日時がありません")
                timestamp = normalize_timestamp(timestamp)
                latest_timestamp = max(latest_timestamp or timestamp, timestamp)
                mountains[osm_id] = {
                    "id": str(osm_id),
                    "type": "peak",
                    "osmId": osm_id,
                    "name": name,
                    "latitude": lat,
                    "longitude": lon,
                    "elevationM": parse_elevation(tags.get("ele")),
                    "nameReading": tags.get("name:ja-Hira", "").strip() or None,
                    "aliases": read_aliases(tags, name),
                    "wikipediaUrl": wikipedia_url(tags.get("wikipedia")),
                    "wikidataUrl": wikidata_url(tags.get("wikidata")),
                }
                if len(mountains) % 5000 == 0:
                    logging.info("名前付きの山頂を %s 件読み取りました", format(len(mountains), ","))
        root.clear()
    if not mountains:
        raise ValueError("配布できる山頂がありません")
    logging.info("読み取り完了: %s 件", format(len(mountains), ","))
    logging.info("収録する山頂の最新編集日時: %s", latest_timestamp)
    return [mountains[key] for key in sorted(mountains)], latest_timestamp


def normalize_timestamp(value):
    stamp = datetime.fromisoformat(value.strip().replace("Z", "+00:00"))
    if stamp.tzinfo is None:
        raise ValueError("元データの日付にタイムゾーンがありません")
    return stamp.astimezone(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def write_distribution(mountains, output_dir, version, source_timestamp, latest_mountain_timestamp, *, source_url=SOURCE_URL, channel="stable", tag_sources=None):
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", version):
        raise ValueError("版は英数字・ピリオド・ハイフン・下線で指定してください")
    target_url = download_url(version, channel)
    source_timestamp = normalize_timestamp(source_timestamp)
    latest_mountain_timestamp = normalize_timestamp(latest_mountain_timestamp)
    if latest_mountain_timestamp > source_timestamp:
        raise ValueError("山頂の最終編集日時が元データの基準日時より新しくなっています")
    validate_tags(mountains)
    validate_graphics(mountains)
    logging.info("%s 件を JSON に変換しています", format(len(mountains), ","))
    raw = (json.dumps(mountains, ensure_ascii=False, separators=(",", ":"), allow_nan=False) + "\n").encode("utf-8")
    # ファイル名と生成時刻をヘッダーに含めず、同じ内容の圧縮結果をそろえる。
    output_dir = Path(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=output_dir) as temporary:
        archive = Path(temporary) / FILE_NAME
        logging.info("gzip 圧縮を開始します（圧縮前 %s バイト）", format(len(raw), ","))
        with archive.open("wb") as stream:
            with gzip.GzipFile(filename="", mode="wb", fileobj=stream, mtime=0) as compressed:
                compressed.write(raw)
        size = archive.stat().st_size
        logging.info("gzip 圧縮完了: %s バイト", format(size, ","))
        if size > MAX_SIZE_BYTES:
            raise ValueError(f"圧縮後のサイズが {MAX_SIZE_BYTES} バイトを超えました。分割を検討してください")
        logging.info("SHA-256 を計算し、manifest を作成しています")
        manifest = {
            "schemaVersion": 5,
            "dataSchemaVersion": 5,
            "name": "山頂",
            "version": version,
            "fileName": FILE_NAME,
            "downloadUrl": target_url,
            "sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
            "sizeBytes": size,
            "uncompressedSizeBytes": len(raw),
            "pointCount": len(mountains),
            "sourceTimestamp": source_timestamp,
            "latestPointTimestamp": latest_mountain_timestamp,
            "sourceUrl": source_url,
            "license": "ODbL-1.0",
            "attribution": "© OpenStreetMap contributors",
        }
        if tag_sources:
            manifest["tagSources"] = tag_sources
        manifest_path = Path(temporary) / "manifest.json"
        manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
        archive.replace(output_dir / FILE_NAME)
        manifest_path.replace(output_dir / "manifest.json")
    logging.info("配布ファイルの保存完了: %s", output_dir)
    return manifest


def verified_source_url(pbf):
    """保存済みの取得記録と PBF が一致することを確かめる。"""
    pbf = Path(pbf)
    path = Path(str(pbf) + ".source.json")
    if not path.exists():
        raise ValueError(f"取得記録がありません。download_source.py で取得してください: {path}")
    source = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(source, dict):
        raise ValueError("取得記録がオブジェクトではありません")
    url = source.get("url")
    if not isinstance(url, str) or not re.fullmatch(r"https://download\.geofabrik\.de/asia/japan-[0-9]{6}\.osm\.pbf", url):
        raise ValueError("取得記録の URL が日付付き全国 PBF ではありません")
    if type(source.get("sizeBytes")) is not int or pbf.stat().st_size != source["sizeBytes"]:
        raise ValueError("取得記録と PBF のサイズが一致しません")
    with pbf.open("rb") as stream:
        if hashlib.file_digest(stream, "md5").hexdigest() != source.get("md5"):
            raise ValueError("取得記録と PBF の MD5 が一致しません")
    logging.info("取得記録と PBF の照合完了: %s", url)
    return url


def build(pbf, output_dir, version, *, channel="stable", graphics_directory=None, graphics_map=None, tags_directory=None, supplements_path=DEFAULT_SUPPLEMENTS):
    started = time.monotonic()
    source_url = verified_source_url(pbf)
    logging.info("全国データの生成を開始します: %s（版 %s）", pbf, version)
    logging.info("元データの日時を確認しています")
    # ダウンロード時刻ではなく、元の PBF が収録している OSM の日時を使う。
    timestamp = subprocess.check_output(
        ["osmium", "fileinfo", "-g", "header.option.osmosis_replication_timestamp", str(pbf)],
        text=True,
    ).strip()
    timestamp = normalize_timestamp(timestamp)
    logging.info("元データの日時: %s", timestamp)
    with tempfile.TemporaryDirectory() as temporary:
        extracted = Path(temporary) / "mountains.osm"
        logging.info("osmium で山頂・火山ノードを抽出しています")
        subprocess.run(
            ["osmium", "tags-filter", str(pbf), "n/natural=peak,volcano", "--omit-referenced", "--progress", "-o", str(extracted)],
            check=True,
        )
        logging.info("osmium の抽出完了")
        mountains, latest_timestamp = read_mountains(extracted)
        # 画像も元データに含め、補足適用前の値を別添付として保存する。
        supplements = read_supplements(supplements_path) if tags_directory is None else None
        tag_sources = ([{"name": name, **metadata} for name, metadata in supplements.get("sources", {}).items()]
                       if supplements is not None else apply_tag_jsons(mountains, tags_directory))
        if graphics_map is not None:
            mappings = json.loads(Path(graphics_map).read_text(encoding="utf-8"))
            if not isinstance(mappings, dict) or not all(isinstance(key, str) and isinstance(value, str) for key, value in mappings.items()):
                raise ValueError("画像の対応表は地点 id と assetId の対応で指定してください")
            for mountain in mountains:
                if mountain["id"] in mappings:
                    asset_id = mappings[mountain["id"]]
                    if not re.fullmatch(r"[a-z][a-z0-9_-]*", asset_id) or graphics_directory is None:
                        raise ValueError("画像の対応表と SVG の保存先を確認してください")
                    mountain["graphic"] = {"svg": (Path(graphics_directory) / f"{asset_id}.svg").read_text(encoding="utf-8")}
        original = mountains
        if supplements is not None:
            mountains = apply_supplements(original, supplements)
        manifest = write_distribution(mountains, output_dir, version, timestamp, latest_timestamp, source_url=source_url, channel=channel, tag_sources=tag_sources)
        if supplements is not None:
            write_companions(output_dir, original, supplements, manifest)
    logging.info("全国データの生成完了（経過 %.1f 秒）", time.monotonic() - started)
    return manifest


def main():
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(message)s", datefmt="%H:%M:%S")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pbf", type=Path)
    parser.add_argument("--output-dir", type=Path, default=Path("dist"))
    parser.add_argument("--version", required=True)
    parser.add_argument("--channel", choices=("stable", "dev"), default=os.environ.get("RELEASE_CHANNEL", "stable"))
    parser.add_argument("--graphics-directory", type=Path)
    parser.add_argument("--graphics-map", type=Path)
    parser.add_argument("--supplements", type=Path, default=DEFAULT_SUPPLEMENTS)
    args = parser.parse_args()
    manifest = build(args.pbf, args.output_dir, args.version, channel=args.channel, graphics_directory=args.graphics_directory, graphics_map=args.graphics_map, supplements_path=args.supplements)
    print(json.dumps(manifest, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
