"""共通形式の地点 Release を再検証し、データの種類によらず公開する。"""

import argparse
import gzip
import hashlib
import json
import math
import os
from pathlib import Path
import re
import sys
import tempfile
import zlib

from release_tools.publish_reviewed import ROOT, check_branch, dataset_for_tag, find_release, version_for_tag

# 地点の形式検証と Pages の処理は既存の共通実装を使う。
sys.path.insert(0, str(ROOT / "points/osm-peaks"))
from release_tools import release_data as site
from scripts.check_release import source_time, MAX_UNCOMPRESSED_BYTES
from scripts.build_data import MAX_SIZE_BYTES
from scripts.distribution_parts import filenames, validate_parts
from scripts.point_document import rows as document_rows
from scripts.point_tags import validate_tags, validate_alias_readings
from scripts.graphics import validate_graphics
from document_metadata import validate_metadata, name_reading, metadata_maps
from distribution_license import license_url


def release_tag(dataset, version, channel):
    return site.release_tag(version, channel, dataset=dataset)


def manifest_path(dataset, channel):
    return site.manifest_path(channel, dataset=dataset)


def validate(directory, dataset, tag, channel):
    directory = Path(directory)
    manifest = json.loads((directory / "manifest.json").read_bytes())
    if not isinstance(manifest, dict):
        raise ValueError("manifest がオブジェクトではありません")
    if type(manifest.get("schemaVersion")) is not int:
        raise ValueError("manifest の版が不正です")
    if manifest["schemaVersion"] != 6:
        # 共通形式に移る前の版は当時の検証を保つ。
        from scripts.check_release import validate as validate_legacy
        return validate_legacy(directory, tag=tag, channel=channel)
    if (manifest.get("schemaFormat") != "yamamuki-manifest"
            or manifest.get("dataSchemaFormat") != "yamamuki-points"
            or type(manifest.get("dataSchemaVersion")) is not int
            or manifest["dataSchemaVersion"] != 6):
        raise ValueError("未対応の地点 manifest の形式です")
    if dataset_for_tag(tag, channel, manifest.get("version")) != dataset:
        raise ValueError("Release のタグと manifest の版が一致しません")
    filename = manifest.get("fileName")
    if not isinstance(filename, str) or not re.fullmatch(r"[a-z][a-z0-9_-]*\.json\.gz", filename):
        raise ValueError("配布ファイル名が不正です")
    expected_url = f"https://github.com/{os.environ['GH_REPO']}/releases/download/{tag}/{filename}"
    if manifest.get("downloadUrl") != expected_url:
        raise ValueError("データの取得先が Release と一致しません")
    if "licenseUrl" in manifest and manifest["licenseUrl"] != license_url(expected_url):
        raise ValueError("利用条件のファイルの URL が Release と一致しません")
    if not isinstance(manifest.get("name"), str) or not manifest["name"].strip():
        raise ValueError("データセットの表示名がありません")
    if any(key in manifest for key in ("sources", "tags", "pointCount", "sourceUrl", "license", "attribution", "supplementSources", "sourceTimestamp", "latestPointTimestamp")):
        raise ValueError("共通情報は metadata にまとめてください")
    metadata = manifest.get("metadata")
    if not isinstance(metadata, dict) or "schemaVersion" in metadata:
        raise ValueError("manifest の metadata が不正です")
    validate_metadata(metadata)
    if not metadata.get("sources"):
        raise ValueError("出典情報がありません")
    stamp = manifest.get("dataTimestamp")
    if not isinstance(stamp, str) or not re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z", stamp):
        raise ValueError("dataTimestamp は UTC の生成日時にしてください")
    source_time(stamp)
    for source in metadata_maps(metadata).get("sources", {}).values():
        if "latestTimestamp" in source and source_time(source["latestTimestamp"]) > source_time(stamp):
            raise ValueError("地点の最終編集日時がデータの生成日時より新しくなっています")
    for key in ("sizeBytes", "uncompressedSizeBytes"):
        if type(manifest.get(key)) is not int or manifest[key] <= 0:
            raise ValueError("データのサイズが不正です")
    if manifest["sizeBytes"] > MAX_SIZE_BYTES or manifest["uncompressedSizeBytes"] > MAX_UNCOMPRESSED_BYTES:
        raise ValueError("データがサイズの上限を超えています")
    archive_path = directory / filename
    if archive_path.stat().st_size != manifest["sizeBytes"]:
        raise ValueError("gzip のサイズが manifest と一致しません")
    if hashlib.sha256(archive_path.read_bytes()).hexdigest() != manifest.get("sha256"):
        raise ValueError("SHA-256 が一致しません")
    with gzip.open(archive_path, "rb") as stream:
        raw = stream.read(MAX_UNCOMPRESSED_BYTES + 1)
    if len(raw) != manifest["uncompressedSizeBytes"]:
        raise ValueError("展開後のサイズが manifest と一致しません")
    body = json.loads(raw)
    rows = document_rows(body, 6)
    if body["metadata"] != metadata or not rows or metadata.get("pointCount") != len(rows):
        raise ValueError("地点 JSON と manifest の metadata または件数が一致しません")
    seen = set()
    for row in rows:
        identifier = row["id"]
        if identifier in seen:
            raise ValueError("地点 ID が重複しています")
        seen.add(identifier)
        if not isinstance(row.get("name"), str) or not row["name"].strip():
            raise ValueError("地点名がありません")
        if row.get("type") is not None and (not isinstance(row["type"], str) or not re.fullmatch(r"[a-z][a-z0-9_]*", row["type"])):
            raise ValueError("地点の種別が不正です")
        for key, limit in (("latitude", 90), ("longitude", 180)):
            value = row.get(key)
            if type(value) not in (int, float) or not math.isfinite(value) or abs(value) > limit:
                raise ValueError("地点の座標が不正です")
        elevation = row.get("elevationM")
        if elevation is not None and (type(elevation) not in (int, float) or not math.isfinite(elevation)):
            raise ValueError("標高の形式が不正です")
        if row.get("nameReading") is not None and not name_reading(row["nameReading"]):
            raise ValueError("読みの形式が不正です")
    validate_tags(rows)
    validate_alias_readings(rows)
    validate_graphics(rows)
    validate_parts(directory, manifest, rows)
    return manifest, rows


class DifferentDataset(ValueError):
    """タグの接頭辞が似ていても、manifest が示す版は別のデータである。"""


def fetch(dataset, tag, directory, channel):
    version_for_tag(dataset, tag, channel)
    directory = Path(directory)
    site.gh("release", "download", tag, "--pattern", "manifest.json", "--dir", str(directory))
    manifest = json.loads((directory / "manifest.json").read_bytes())
    if not isinstance(manifest, dict):
        raise ValueError("manifest がオブジェクトではありません")
    if dataset_for_tag(tag, channel, manifest.get("version")) != dataset:
        raise DifferentDataset("Release のデータ ID が一致しません")
    assets = json.loads(site.gh("release", "view", tag, "--json", "assets"))["assets"]
    selected = data_assets(manifest, {asset["name"] for asset in assets})
    site.gh("release", "download", tag, *[part for name in selected for part in ("--pattern", name)], "--dir", str(directory))
    return validate(directory, dataset, tag, channel)


def data_assets(manifest, names):
    filename = manifest.get("fileName", "")
    if not isinstance(filename, str) or not re.fullmatch(r"[a-z][a-z0-9_-]*\.json\.gz", filename):
        raise ValueError("配布ファイル名が不正です")
    parts = filenames(filename.removesuffix(".json.gz"))
    selected = [filename]
    if any(name in names for name in parts):
        if not all(name in names for name in parts):
            raise ValueError("オリジナル・補足と manifest がそろっていません")
        selected.extend(parts)
    if manifest.get("schemaVersion", 0) < 6:
        # 旧版の添付ファイルも既存の検証処理で確かめる。
        from scripts.supplements import COMPANION_FILES, LEGACY_COMPANION_FILE
        selected.extend(name for name in (*COMPANION_FILES, LEGACY_COMPANION_FILE) if name in names and name not in selected)
    return selected


def previous_release(dataset, channel, directory, releases):
    if channel == "stable":
        candidates = []
        for release in releases:
            if release["draft"] or release["prerelease"] or site.needs_review(release):
                continue
            try:
                tag = release["tag_name"]
                candidates.append((version_for_tag(dataset, tag, channel), tag))
            except ValueError:
                continue
        for index, (_, tag) in enumerate(sorted(candidates, reverse=True)):
            try:
                return fetch(dataset, tag, directory / str(index), channel)
            except DifferentDataset:
                continue
        return None
    catalog = site.read_catalog()
    manifest = catalog.get(manifest_path(dataset, channel)) if catalog is not None else None
    if manifest is None:
        return None
    tag = release_tag(dataset, manifest["version"], channel)
    release = next((r for r in releases if r["tag_name"] == tag), None)
    if release is None:
        raise site.MissingPreviousRelease("前回の公開 Release がありません。初期化を指定してください")
    if release["draft"] or not release["prerelease"]:
        raise ValueError("参照先が公開済みの開発版ではありません")
    previous = fetch(dataset, tag, directory, channel)
    if previous[0] != manifest:
        raise ValueError("最新版の参照先と Release の manifest が一致しません")
    return previous


def publish(dataset, tag, channel, sha256="", reason=""):
    check_branch(channel)
    version_for_tag(dataset, tag, channel)
    releases = site.releases()
    matches = [r for r in releases if r["tag_name"] == tag]
    if len(matches) != 1 or matches[0]["prerelease"] != (channel == "dev"):
        raise ValueError("Release または配布先が一致しません")
    release = matches[0]
    checksum = sha256.strip() or site.reviewed_checksum(release)
    if not re.fullmatch(r"[0-9a-f]{64}", checksum):
        raise ValueError("確認した SHA-256 が不正です")
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        current = fetch(dataset, tag, root / "current", channel)
        if current[0]["sha256"] != checksum:
            raise ValueError("確認したファイルと SHA-256 が異なります")
        warning = ""
        try:
            previous = previous_release(dataset, channel, root / "previous", releases)
        except site.MissingPreviousRelease:
            if os.environ.get("PAGES_INITIALIZE") != "true":
                raise
            previous = None
            warning = "前回の Release がないため、初期化を指定した手動公開で配布を再開します"
        except (RuntimeError, ValueError, OSError, KeyError, TypeError, EOFError, zlib.error):
            if release["draft"]:
                raise
            previous = None
            warning = "前回の参照先を取得できないため、確認した公開済みの版で参照先を復旧します"
        contents = review_notes(release, current, checksum, previous, warning, reason)
        notes = root / "notes.md"
        site.write_report(notes, contents)
        if release["draft"]:
            site.gh("release", "edit", tag, "--notes-file", str(notes))
            site.gh("release", "edit", tag, "--draft=false", "--latest=false")
        site.update_latest(root, current[0], channel, dataset=dataset)
        site.append_summary(f"\n## 公開したリリース\n\n[公開したリリースを開く](https://github.com/{os.environ['GH_REPO']}/releases/tag/{tag})\n")


def review_notes(release, current, checksum, previous, warning, reason):
    original_notes = release.get("body") or ""
    if site.needs_review(release):
        original_notes = original_notes.replace(site.REVIEW_HEADING, "## 生成時の要確認事項（手動確認済み）", 1)
    contents = original_notes + "\n\n## データの検査結果（公開前の再検査）\n\n"
    contents += f"- 地点数: {len(current[1]):,}\n- 再検証した SHA-256: `{checksum}`\n"
    if previous is not None:
        contents += f"- 前回からの地点数の増減: {len(current[1]) - len(previous[1]):+,}\n"
    if warning:
        contents += f"- {warning}\n"
    contents += f"\n## 手動公開の確認\n\n確認者: {os.environ.get('GITHUB_ACTOR', '不明')}\n\n{reason.strip() or '理由の記入なし'}\n"
    return contents


BACKUP_PREFIX = "promotion-backup-"


def download_assets(tag, directory, names):
    site.gh("release", "download", tag, *[part for name in names for part in ("--pattern", name)],
            "--dir", str(directory))


def retarget_manifest(path, source_tag, stable_tag):
    manifest = json.loads(path.read_bytes())
    if not isinstance(manifest, dict):
        raise ValueError(f"添付 manifest がオブジェクトではありません: {path.name}")
    filename = manifest.get("fileName")
    base = f"https://github.com/{os.environ['GH_REPO']}/releases/download"
    old_url = f"{base}/{source_tag}/{filename}"
    new_url = f"{base}/{stable_tag}/{filename}"
    current_url = manifest.get("downloadUrl")
    if current_url not in (old_url, new_url):
        raise ValueError(f"添付 manifest の取得先が下書きと一致しません: {path.name}")
    if "licenseUrl" in manifest:
        if manifest["licenseUrl"] != license_url(current_url):
            raise ValueError(f"利用条件の取得先が一致しません: {path.name}")
        manifest["licenseUrl"] = license_url(new_url)
    if current_url == new_url:
        return False
    manifest["downloadUrl"] = new_url
    path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    return True


def replace_manifests(tag, directory, replacements, assets):
    # 元の添付を退避してから更新する。中断した場合は退避した添付から再開する。
    backups = []
    for name in replacements:
        backup = BACKUP_PREFIX + name
        existing = assets.get(name)
        if backup in assets:
            backups.append(assets[backup]["id"])
            if existing is not None:
                site.gh("api", "--method", "DELETE", site.endpoint(f"releases/assets/{existing['id']}"))
        else:
            asset_id = existing["id"]
            site.gh("api", "--method", "PATCH", site.endpoint(f"releases/assets/{asset_id}"), "-f", f"name={backup}")
            backups.append(asset_id)
        site.gh("release", "upload", tag, str(directory / name))
    if replacements:
        checked = directory / "checked"
        download_assets(tag, checked, replacements)
        for name in replacements:
            if (checked / name).read_bytes() != (directory / name).read_bytes():
                raise ValueError(f"正式版用の添付 manifest が一致しません: {name}")
    for asset_id in backups:
        site.gh("api", "--method", "DELETE", site.endpoint(f"releases/assets/{asset_id}"))


def formalize(target, sha256="", reason=""):
    check_branch("stable")
    releases = site.releases()
    release = find_release(target, releases)
    tag = release["tag_name"]
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", tag):
        raise ValueError("タグの形式が不正です")
    if release["draft"] != release["prerelease"]:
        raise ValueError("正式公開には dev の下書き、参照先の復旧には公開済みの正式版を指定してください")

    with tempfile.TemporaryDirectory() as temporary:
        directory = Path(temporary)
        assets = {asset["name"]: asset for asset in release["assets"]}
        backup = BACKUP_PREFIX + "manifest.json"
        primary = backup if backup in assets else "manifest.json"
        download_assets(tag, directory, [primary])
        manifest = json.loads((directory / primary).read_bytes())
        if not isinstance(manifest, dict):
            raise ValueError("manifest がオブジェクトではありません")
        channel = "dev" if release["draft"] else "stable"
        version = manifest.get("version")
        dataset = dataset_for_tag(tag, channel, version)
        if channel == "stable":
            publish(dataset, tag, channel, sha256, reason)
            return
        if manifest.get("schemaVersion") != 6:
            raise ValueError("正式公開には共通形式の地点 manifest が必要です")

        stable_tag = release_tag(dataset, version, "stable")
        refs = site.gh("api", site.endpoint(f"git/matching-refs/tags/{stable_tag}"), "--jq", ".[].ref").splitlines()
        if f"refs/tags/{stable_tag}" in refs or any(item["tag_name"] == stable_tag for item in releases):
            raise ValueError(f"正式版のタグは既に使われています: {stable_tag}")

        # 退避中の manifest も元の名前で扱い、添付データのハッシュと照合する。
        names = {name.removeprefix(BACKUP_PREFIX) for name in assets}
        selected = data_assets(manifest, names)
        if "licenseUrl" in manifest and "LICENSE.txt" not in assets:
            raise ValueError("利用条件の添付ファイルがありません")
        manifests = ["manifest.json", *[name for name in selected if name.endswith("-manifest.json")]]
        originals = {name: BACKUP_PREFIX + name if BACKUP_PREFIX + name in assets else name for name in manifests}
        download_assets(tag, directory, [originals.get(name, name) for name in selected])
        replacements = []
        for name, original in originals.items():
            if original != name:
                (directory / original).replace(directory / name)
            if retarget_manifest(directory / name, tag, stable_tag) or original != name:
                replacements.append(name)
        current = validate(directory, dataset, stable_tag, "stable")
        checksum = sha256.strip() or site.reviewed_checksum(release)
        if checksum != current[0]["sha256"]:
            raise ValueError("確認したファイルと SHA-256 が異なります")
        previous = previous_release(dataset, "stable", directory / "previous", releases)
        contents = review_notes(release, current, checksum, previous, "", reason)

        commit = release["target_commitish"]
        if not re.fullmatch(r"[0-9a-f]{40}", commit):
            raise ValueError("下書きの作成元コミットが SHA で固定されていません")
        replace_manifests(tag, directory, replacements, assets)
        request = directory / "publish.json"
        request.write_text(json.dumps({
            "tag_name": stable_tag, "target_commitish": commit, "name": stable_tag, "body": contents,
            "draft": False, "prerelease": False, "make_latest": "false",
        }), encoding="utf-8")
        published = json.loads(site.gh("api", "--method", "PATCH", site.endpoint(f"releases/{release['id']}"),
                                       "--input", str(request)))
        if (published["id"] != release["id"] or published["tag_name"] != stable_tag
                or published["draft"] or published["prerelease"]):
            raise ValueError("下書き Release が正式版として公開されたことを確認できません")
        site.append_summary(f"\n## 正式公開した Release\n\n[Release を開く]({published['html_url']})\n")
        site.update_latest(directory, current[0], "stable", dataset=dataset)


def main():
    if len(sys.argv) == 3 and sys.argv[1] == "--verify-pages":
        site.verify_pages(Path(sys.argv[2]))
        return
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dataset", required=True)
    parser.add_argument("--channel", required=True, choices=("stable", "dev"))
    parser.add_argument("--tag", required=True)
    parser.add_argument("--sha256", default="")
    parser.add_argument("--reason", default="")
    parser.add_argument("--manual", action="store_true", required=True)
    args = parser.parse_args()
    publish(args.dataset, args.tag, args.channel, args.sha256, args.reason)


if __name__ == "__main__":
    main()
