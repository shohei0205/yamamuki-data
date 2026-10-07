"""架空の地点データを開発版だけに配布する。"""

import argparse
from datetime import datetime, timezone
import gzip
import hashlib
import json
import math
import os
from pathlib import Path
import re
import sys


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "points/osm_peaks"))
from scripts import release_data
from scripts.graphics import validate_graphics
from scripts.point_tags import validate_tags


def validate_rows(rows):
    if not isinstance(rows, list) or not rows:
        raise ValueError("テスト地点がありません")
    seen = set()
    for row in rows:
        if not isinstance(row, dict):
            raise ValueError("地点がオブジェクトではありません")
        identifier = row.get("id")
        if not isinstance(identifier, str) or not identifier.strip() or identifier in seen:
            raise ValueError("地点 ID が不正または重複しています")
        seen.add(identifier)
        if not isinstance(row.get("name"), str) or not row["name"].strip():
            raise ValueError("地点名がありません")
        if row.get("type") is not None and (not isinstance(row["type"], str) or not re.fullmatch(r"[a-z][a-z0-9_]*", row["type"])):
            raise ValueError("地点の種別が不正です")
        for key, limit in (("latitude", 90), ("longitude", 180)):
            value = row.get(key)
            if type(value) not in (int, float) or not math.isfinite(value) or abs(value) > limit:
                raise ValueError("座標が不正です")
    validate_tags(rows)
    validate_graphics(rows)


def generate(version, directory):
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", version):
        raise ValueError("版の形式が不正です")
    rows = json.loads((ROOT / "points/testdata/points.json").read_text(encoding="utf-8"))
    validate_rows(rows)
    raw = (json.dumps(rows, ensure_ascii=False, separators=(",", ":")) + "\n").encode("utf-8")
    archive = gzip.compress(raw, mtime=0)
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    filename = "test-points.json.gz"
    tag = f"testdata-dev-{version}"
    manifest = {"schemaVersion": 5, "dataSchemaVersion": 5, "name": "テスト用地点（架空）",
                "version": version, "fileName": filename,
                "downloadUrl": f"https://github.com/{os.environ.get('GH_REPO', 'shohei0205/yamamuki-data')}/releases/download/{tag}/{filename}",
                "sha256": hashlib.sha256(archive).hexdigest(), "sizeBytes": len(archive),
                "uncompressedSizeBytes": len(raw), "pointCount": len(rows),
                "license": "CC0-1.0", "attribution": "山むきの動作確認用に作成した架空データ"}
    (directory / filename).write_bytes(archive)
    (directory / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    return manifest


def publish(version, directory):
    # 手元からの誤実行も含め、dev 以外では公開しない。
    if os.environ.get("GITHUB_REF") != "refs/heads/dev":
        raise ValueError("テストデータの公開は dev ブランチ限定です")
    release_data.endpoint("")
    catalog = release_data.read_catalog()
    if catalog is None:
        raise ValueError("既存の公開一覧がないため公開を停止します。通常の手動公開で初期化してください")
    manifest = generate(version, directory)
    directory = Path(directory)
    tag = f"testdata-dev-{version}"
    notes = directory / "notes.md"
    notes.write_text("[Codex] アプリの表示確認用の架空データです。登山・位置判断には使用しないでください。\n", encoding="utf-8")
    release_data.gh("release", "create", tag, str(directory / manifest["fileName"]), str(directory / "manifest.json"),
                    "--draft", "--prerelease", "--latest=false", "--target", os.environ["GITHUB_SHA"],
                    "--title", "テスト用地点（開発版） " + version, "--notes-file", str(notes))
    # アップロードしたデータを取り直し、同じファイルであることを公開前に確認する。
    checked = directory / "checked"
    release_data.gh("release", "download", tag, "--pattern", "manifest.json", "--pattern", manifest["fileName"], "--dir", str(checked))
    actual = (checked / manifest["fileName"]).read_bytes()
    if (json.loads((checked / "manifest.json").read_text(encoding="utf-8")) != manifest
            or len(actual) != manifest["sizeBytes"] or hashlib.sha256(actual).hexdigest() != manifest["sha256"]):
        raise ValueError("添付ファイルが生成結果と一致しません")
    raw = gzip.decompress(actual)
    if len(raw) != manifest["uncompressedSizeBytes"] or len(json.loads(raw)) != manifest["pointCount"]:
        raise ValueError("展開後のサイズ・件数が一致しません")
    validate_rows(json.loads(raw))
    manifests = dict(catalog)
    manifests["points/testdata-dev/manifest.json"] = manifest
    histories = {key: list(value) for key, value in getattr(catalog, "histories", {}).items()}
    histories.setdefault("points/testdata-dev/history.json", []).append({
        "kind": "publication", "publishedAt": datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z"),
        "version": version, "releaseUrl": f"https://github.com/{os.environ['GH_REPO']}/releases/tag/{tag}",
        "downloadUrl": manifest["downloadUrl"],
        "actionsRunUrl": f"https://github.com/{os.environ['GH_REPO']}/actions/runs/{os.environ['GITHUB_RUN_ID']}/attempts/{os.environ.get('GITHUB_RUN_ATTEMPT', '1')}"})
    release_data.write_site(manifests, histories)
    release_data.gh("release", "edit", tag, "--draft=false", "--latest=false")
    release_data.append_summary("\n## テスト用地点\n\n開発版だけに公開します。正式版の一覧は維持します。\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("generate", "publish"))
    parser.add_argument("--version", required=True)
    parser.add_argument("--directory", type=Path, default=ROOT / "points/testdata/dist")
    args = parser.parse_args()
    (publish if args.command == "publish" else generate)(args.version, args.directory)


if __name__ == "__main__":
    main()
