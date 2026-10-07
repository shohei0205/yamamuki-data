"""配布ファイルの整合性と、前回公開版からの件数の変化を確かめる。"""

from collections import Counter
from datetime import datetime
import gzip
import hashlib
import json
import math
from pathlib import Path
import re

from scripts.build_data import FILE_NAME, MAX_SIZE_BYTES
from scripts.graphics import validate_graphics
from scripts.point_tags import validate_tags
from scripts.release_channels import download_url, release_tag


MIN_COUNT = 10_000
MIN_CELL_COUNT = 20
MAX_UNCOMPRESSED_BYTES = 100_000_000


def source_time(value):
    if not isinstance(value, str):
        raise ValueError("元データの日時が文字列ではありません")
    stamp = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if stamp.tzinfo is None:
        raise ValueError("元データの日時にタイムゾーンがありません")
    return stamp


def validate(directory, *, tag=None, channel="stable"):
    """手動公開でも省略しない検査。破損や不正な形式は例外にする。"""
    directory = Path(directory)
    manifest = json.loads((directory / "manifest.json").read_text(encoding="utf-8"))
    if not isinstance(manifest, dict):
        raise ValueError("manifest がオブジェクトではありません")
    count_key = "pointCount" if manifest.get("schemaVersion") == 5 else "mountainCount"
    for key in (count_key, "sizeBytes", "uncompressedSizeBytes"):
        if type(manifest.get(key)) is not int or manifest[key] <= 0:
            raise ValueError(f"{key} は正の整数で指定してください")
    if type(manifest.get("schemaVersion")) is not int or manifest["schemaVersion"] not in (1, 2, 3, 4, 5):
        raise ValueError("未対応の schemaVersion です")
    # 旧 manifest は当時の山頂形式の版を使う。明示された版は独立して検査する。
    data_version = manifest.get("dataSchemaVersion", manifest["schemaVersion"])
    if type(data_version) is not int or data_version not in (1, 2, 3, 4, 5):
        raise ValueError("未対応の dataSchemaVersion です")
    if "name" in manifest and (not isinstance(manifest["name"], str) or not manifest["name"].strip()):
        raise ValueError("データセットの表示名が不正です")
    version = manifest.get("version")
    if not isinstance(version, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", version):
        raise ValueError("版の形式が不正です")
    if tag is not None and tag != release_tag(version, channel):
        raise ValueError("Release のタグと manifest の版が一致しません")
    if manifest.get("fileName") != FILE_NAME:
        raise ValueError("配布ファイル名が不正です")
    if manifest["schemaVersion"] >= 4 or "downloadUrl" in manifest:
        if manifest.get("downloadUrl") != download_url(version, channel):
            raise ValueError("データ本体の取得 URL が Release の版や配布先と一致しません")
    source_time(manifest["sourceTimestamp"])
    if manifest["schemaVersion"] >= 3:
        latest = source_time(manifest.get("latestPointTimestamp" if manifest["schemaVersion"] >= 5 else "latestMountainTimestamp"))
        if latest > source_time(manifest["sourceTimestamp"]):
            raise ValueError("山頂の最終編集日時が元データの基準日時より新しくなっています")
    archive_path = directory / FILE_NAME
    if not 0 < archive_path.stat().st_size <= MAX_SIZE_BYTES:
        raise ValueError("gzip のサイズが許容範囲外です")
    archive = archive_path.read_bytes()
    if len(archive) != manifest.get("sizeBytes"):
        raise ValueError("gzip のサイズが manifest と一致しません")
    if hashlib.sha256(archive).hexdigest() != manifest.get("sha256"):
        raise ValueError("SHA-256 が一致しません")
    with gzip.open(archive_path, "rb") as stream:
        raw = stream.read(MAX_UNCOMPRESSED_BYTES + 1)
    if len(raw) > MAX_UNCOMPRESSED_BYTES or len(raw) != manifest.get("uncompressedSizeBytes"):
        raise ValueError("展開後のサイズが不正です")
    rows = json.loads(raw)
    if not isinstance(rows, list) or not rows or len(rows) != manifest.get(count_key):
        raise ValueError("山の件数が不正です")
    seen = set()
    for row in rows:
        if not isinstance(row, dict):
            raise ValueError("山データがオブジェクトではありません")
        if data_version >= 5:
            identifier = row.get("id")
            valid_id = isinstance(identifier, str) and re.fullmatch(r"[1-9][0-9]*", identifier)
        else:
            identifier = row.get("osmId")
            valid_id = type(identifier) is int and identifier > 0
        if not valid_id or identifier in seen:
            raise ValueError("山の ID が不正または重複しています")
        seen.add(identifier)
        if data_version >= 5 and "type" in row:
            if not isinstance(row["type"], str) or not re.fullmatch(r"[a-z][a-z0-9_]*", row["type"]):
                raise ValueError("地点の種別の形式が不正です")
        if data_version >= 5 and "osmId" in row:
            osm_id = row["osmId"]
            if type(osm_id) is not int or osm_id <= 0:
                raise ValueError("osmId は正の整数で指定してください")
            if identifier != str(osm_id):
                raise ValueError("山頂の id と osmId が一致しません")
        if not isinstance(row.get("name"), str) or not row["name"].strip():
            raise ValueError("山の名前がありません")
        for key, limit in (("latitude", 90), ("longitude", 180)):
            value = row.get(key)
            if type(value) not in (int, float) or not math.isfinite(value) or abs(value) > limit:
                raise ValueError("山の座標が不正です")
        elevation = row.get("elevationM")
        if (data_version < 5 and "elevationM" not in row) or (elevation is not None and
                (type(elevation) not in (int, float) or not math.isfinite(elevation))):
            raise ValueError("標高の形式が不正です")
        if data_version >= 2:
            for key in ("nameReading", "wikipediaUrl", "wikidataUrl"):
                if key not in row and data_version >= 5:
                    continue
                if key not in row or (row[key] is not None and not isinstance(row[key], str)):
                    raise ValueError(f"{key} の形式が不正です")
            aliases = row.get("aliases", [] if data_version >= 5 else None)
            if not isinstance(aliases, list) or not all(isinstance(a, str) and a.strip() for a in aliases):
                raise ValueError("別名の形式が不正です")
    validate_tags(rows)
    validate_graphics(rows)
    return manifest, rows


def cells(rows):
    # 行政境界に依存せず、地域単位の欠落を検出するための緯度経度1度の区画。
    return Counter((math.floor(row["latitude"]), math.floor(row["longitude"])) for row in rows)


def assess(current, previous=None):
    manifest, rows = current
    warnings = []
    if len(rows) < MIN_COUNT:
        warnings.append(f"全国の件数が最低目安 {MIN_COUNT:,} 件未満です: {len(rows):,} 件")
    if previous is None:
        warnings.append("初回公開のため、比較対象がありません。手動確認が必要です")
        return warnings
    old_manifest, old_rows = previous
    if len(rows) * 5 <= len(old_rows) * 4:
        warnings.append(f"全国の件数が20%以上減少: {len(old_rows):,} → {len(rows):,} 件 ({1 - len(rows) / len(old_rows):.1%}減)")
    before, after = cells(old_rows), cells(rows)
    for cell, count in sorted(before.items()):
        if count >= MIN_CELL_COUNT and after[cell] * 5 <= count * 4:
            warnings.append(f"区画（北緯{cell[0]}度・東経{cell[1]}度から各1度）が20%以上減少: {count:,} → {after[cell]:,} 件")
    if source_time(manifest["sourceTimestamp"]) < source_time(old_manifest["sourceTimestamp"]):
        warnings.append("元データの日時が前回公開版より古くなっています")
    latest = manifest.get("latestPointTimestamp" if manifest["schemaVersion"] >= 5 else "latestMountainTimestamp")
    old_latest = old_manifest.get("latestPointTimestamp" if old_manifest["schemaVersion"] >= 5 else "latestMountainTimestamp")
    if latest is not None and old_latest is not None and source_time(latest) == source_time(old_latest):
        warnings.append(f"収録山頂の最新編集日時が前回公開版と同じです: {latest}。自動公開せず下書きに残します")
    if manifest["schemaVersion"] != old_manifest["schemaVersion"]:
        warnings.append("schemaVersion が前回公開版と異なります。アプリの対応を確認してください")
    if manifest.get("dataSchemaVersion", manifest["schemaVersion"]) != old_manifest.get("dataSchemaVersion", old_manifest["schemaVersion"]):
        warnings.append("dataSchemaVersion が前回公開版と異なります。アプリの対応を確認してください")
    return warnings


def report(current, previous, warnings):
    manifest, rows = current
    lines = ["## データの検査結果", "", "判定: " + ("手動確認が必要" if warnings else "自動公開可能"), "",
             f"- 今回: {manifest['version']}、{len(rows):,} 件",
             f"- 前回: {previous[0]['version']}、{len(previous[1]):,} 件" if previous else "- 前回: 比較できず",
             f"- 元データの日時: {manifest['sourceTimestamp']}",
             f"- 収録する山頂の最新編集日時: {manifest.get('latestPointTimestamp', manifest.get('latestMountainTimestamp', '旧形式のため記録なし'))}",
             f"- gzip: {manifest['sizeBytes']:,} バイト",
             f"- SHA-256: `{manifest['sha256']}`", "",
             "### 確認事項", "", *([f"- {warning}" for warning in warnings] or ["- 件数と元データの日時に異常はありません"]), "",
             "© OpenStreetMap contributors — [ODbL 1.0](https://opendatacommons.org/licenses/odbl/1-0/)", "",
             "元データ: [Geofabrik](https://download.geofabrik.de/asia/japan.html)", ""]
    return "\n".join(lines)
