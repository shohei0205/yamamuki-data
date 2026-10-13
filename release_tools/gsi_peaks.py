"""コミット済みの国土地理院の地点 JSON を検証し、Release に配布する。"""

import argparse
from collections import defaultdict
import gzip
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil

from release_tools.test_points import ROOT, release_data, validate_rows

DATA = ROOT / "points/gsi-peaks"
FILE = "gsi-peaks.json"
INFO = "snapshot.json"
PROVENANCE = ("license", "attribution", "sourceUrl", "sourceDate", "experimental", "coordinatePolicy", "elevationPolicy")


def validate(points):
    validate_rows(points)
    for point in points:
        if not point["id"].startswith(("gsi-1003:", "gsi-natural:")) or point.get("type") != "peak":
            raise ValueError("国土地理院の山頂以外の地点が含まれています")
        if point.get("nameReading") is not None and not isinstance(point["nameReading"], str):
            raise ValueError("読みの形式が不正です")
        elevation = point.get("elevationM")
        if elevation is not None and (type(elevation) not in (int, float) or not math.isfinite(elevation)):
            raise ValueError("標高の形式が不正です")
        if not isinstance(point.get("aliases"), list) or any(not isinstance(alias, str) for alias in point["aliases"]):
            raise ValueError("別名の形式が不正です")


def snapshot(source, output=DATA):
    """手元で生成した地点と出典の要約だけを、コミット対象の場所へ取り込む。"""
    raw = (source / FILE).read_bytes()
    points = json.loads(raw)
    validate(points)
    manifest = json.loads((source / "manifest.json").read_bytes())
    if manifest["pointCount"] != len(points) or manifest["uncompressedSizeBytes"] != len(raw):
        raise ValueError("生成結果と manifest の件数・サイズが一致しません")
    if gzip.decompress((source / (FILE + ".gz")).read_bytes()) != raw:
        raise ValueError("生成した JSON と gzip の内容が一致しません")
    groups = defaultdict(list)
    for item in manifest["sources"]:
        url = item["url"]
        kind = "natural" if "/experimental_nnfpt/" in url else "dem" if "/xyz/dem/" in url else "mountainList"
        groups[kind].append(item)
    info = {key: manifest[key] for key in PROVENANCE}
    info.update(inputSha256=hashlib.sha256(raw).hexdigest(), pointCount=len(points), sourceSnapshot={})
    for kind, items in sorted(groups.items()):
        dates = [item["fetchedAt"] for item in items]
        summary = {"fileCount": len(items), "firstFetchedAt": min(dates), "lastFetchedAt": max(dates)}
        if kind != "mountainList":
            summary["urlTemplate"] = re.sub(r"/\d+/\d+(\.(?:geojson|txt))$", r"/{x}/{y}\1", items[0]["url"])
        if kind == "mountainList":
            summary["files"] = items
        info["sourceSnapshot"][kind] = summary
    output.mkdir(parents=True, exist_ok=True)
    (output / FILE).write_bytes(raw)
    (output / INFO).write_text(json.dumps(info, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    return info


def prepare(version, directory, channel, source=DATA):
    if channel not in ("stable", "dev") or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", version):
        raise ValueError("配布先または版の形式が不正です")
    repo = os.environ.get("GH_REPO", "shohei0205/yamamuki-data")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo):
        raise ValueError("リポジトリ名が不正です")
    raw = (source / FILE).read_bytes()
    points = json.loads(raw)
    validate(points)
    info = json.loads((source / INFO).read_bytes())
    if info["inputSha256"] != hashlib.sha256(raw).hexdigest() or info["pointCount"] != len(points):
        raise ValueError("地点 JSON と出典情報が一致しません。snapshot コマンドで更新してください")
    if not info.get("license") or not info.get("attribution") or not info.get("sourceSnapshot"):
        raise ValueError("出典・ライセンスがありません")
    tag = f"gsi-peaks-{'dev-' if channel == 'dev' else ''}{version}"
    archive = gzip.compress(raw, mtime=0)
    manifest = {key: info[key] for key in PROVENANCE}
    manifest.update(schemaVersion=5, dataSchemaVersion=5, name="山頂（国土地理院）", version=version,
                    fileName=FILE + ".gz", pointCount=len(points),
                    downloadUrl=f"https://github.com/{repo}/releases/download/{tag}/{FILE}.gz",
                    sha256=hashlib.sha256(archive).hexdigest(), sizeBytes=len(archive),
                    uncompressedSizeBytes=len(raw), inputSha256=info["inputSha256"], sourceSnapshot=info["sourceSnapshot"])
    directory.mkdir(parents=True, exist_ok=True)
    (directory / (FILE + ".gz")).write_bytes(archive)
    (directory / FILE).write_bytes(raw)
    shutil.copyfile(source / INFO, directory / INFO)
    (directory / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    return tag, manifest


def publish(version, directory, source=DATA):
    ref = os.environ.get("GITHUB_REF")
    if ref not in ("refs/heads/main", "refs/heads/dev"):
        raise ValueError("Release の公開は main または dev ブランチ限定です")
    channel = "dev" if ref == "refs/heads/dev" else "stable"
    tag, manifest = prepare(version, directory, channel, source)
    notes = directory / "notes.md"
    notes.write_text("[Codex] コミット済みの国土地理院の地点 JSON を配布します。\n\n"
                     + f"- 地点数: {manifest['pointCount']}\n- SHA-256: `{manifest['sha256']}`\n"
                     + f"- 入力コミット: `{os.environ['GITHUB_SHA']}`\n"
                     + f"- {manifest['attribution']}\n- ライセンス: {manifest['license']}\n", encoding="utf-8", newline="\n")
    names = (FILE, FILE + ".gz", "manifest.json", INFO)
    options = ("--prerelease",) if channel == "dev" else ()
    # 既存タグは create が失敗する。公開済みのアセットを差し替えない。
    release_data.gh("release", "create", tag, *(str(directory / name) for name in names),
                    "--draft", "--latest=false", "--target", os.environ["GITHUB_SHA"],
                    "--title", "国土地理院の山頂 " + version, "--notes-file", str(notes), *options)
    checked = directory / "checked"
    release_data.gh("release", "download", tag, *(part for name in names for part in ("--pattern", name)),
                    "--dir", str(checked))
    for name in names:
        if (checked / name).read_bytes() != (directory / name).read_bytes():
            raise ValueError(f"Release の添付ファイルが一致しません: {name}")
    release_data.gh("release", "edit", tag, "--draft=false", "--latest=false")
    release_data.append_summary(f"\n## 国土地理院の山頂\n\n{manifest['pointCount']:,} 地点を Release `{tag}` に公開しました。\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("snapshot", "prepare", "publish"))
    parser.add_argument("--source", type=Path)
    parser.add_argument("--directory", type=Path, default=ROOT / "build/gsi-release")
    parser.add_argument("--version")
    parser.add_argument("--channel", choices=("stable", "dev"), default="dev")
    args = parser.parse_args()
    if args.command == "snapshot":
        if args.source is None:
            parser.error("snapshot には --source が必要です")
        snapshot(args.source)
    else:
        if args.version is None:
            parser.error("--version が必要です")
        source = args.source or DATA
        if args.command == "publish":
            publish(args.version, args.directory, source)
        else:
            prepare(args.version, args.directory, args.channel, source)


if __name__ == "__main__":
    main()
