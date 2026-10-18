"""下書きの作成と、確認済みの同じファイルの公開を分けて行う。"""

import argparse
import json
import logging
import os
from pathlib import Path
import re
import tempfile
import zlib
from urllib.parse import urlsplit

import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[3]))
from release_tools import release_data as shared

from scripts.build_data import FILE_NAME
from scripts.supplements import COMPANION_FILES, LEGACY_COMPANION_FILE
from scripts.check_release import assess, report, validate
from scripts.release_channels import check_branch, check_tag, prefix, release_tag, download_url


def fetch(tag, directory, channel="stable"):
    check_tag(tag, channel)
    shared.gh("release", "download", tag, "--pattern", "manifest.json", "--pattern", FILE_NAME, "--dir", str(directory))
    assets = json.loads(shared.gh("release", "view", tag, "--json", "assets"))["assets"]
    names = {asset["name"] for asset in assets}
    from scripts.distribution_parts import filenames
    parts = filenames("osm-peaks")
    if any(name in names for name in parts[1:]):
        if not all(name in names for name in parts):
            raise ValueError("別配布データと manifest がそろっていません")
        shared.gh("release", "download", tag, *[v for name in parts for v in ("--pattern", name)], "--dir", str(directory))
    elif any(name in names for name in (*COMPANION_FILES, LEGACY_COMPANION_FILE)):
        if not all(name in names for name in COMPANION_FILES):
            raise ValueError("Release の補足ファイルがそろっていません")
        files = (*COMPANION_FILES, LEGACY_COMPANION_FILE) if LEGACY_COMPANION_FILE in names else COMPANION_FILES
        patterns = [part for name in files for part in ("--pattern", name)]
        shared.gh("release", "download", tag, *patterns, "--dir", str(directory))
    return validate(directory, tag=tag, channel=channel)


def find_release(target, channel="stable"):
    """未公開のタグも扱える一覧 API から、指定した1件だけを選ぶ。"""
    target = target.strip()
    is_url = target.startswith("https://")
    if is_url:
        parsed = urlsplit(target)
        start = f"/{os.environ['GH_REPO']}/releases/tag/"
        if (parsed.netloc != "github.com" or parsed.query or parsed.fragment
                or not parsed.path.startswith(start)
                or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", parsed.path[len(start):])):
            raise ValueError("このリポジトリの Release ページの URL を指定してください")
    else:
        check_tag(target, channel)
    matches = [entry for entry in shared.releases()
               if (entry.get("html_url") == target if is_url else entry["tag_name"] == target)]
    if len(matches) != 1:
        raise ValueError("指定した Release を一意に取得できません。URL・タグと読み取り権限を確認してください")
    check_tag(matches[0]["tag_name"], channel)
    return matches[0]


def manifest_path(channel):
    prefix(channel)
    return ("points/osm-peaks" if channel == "stable" else "points/osm-peaks-dev") + "/manifest.json"


def read_manifest(channel):
    path = manifest_path(channel)
    catalog = shared.read_catalog()
    if catalog is None:
        return None
    return catalog.get(path)


def stable_baseline(directory):
    """正式版は Pages に出さないので、要確認でない最新の正式版 Release と比べる。"""
    candidates = []
    for release in shared.releases():
        if release["draft"] or release["prerelease"] or shared.needs_review(release):
            continue
        try:
            check_tag(release["tag_name"], "stable")
        except ValueError:
            continue
        candidates.append(release["tag_name"])
    # 版名は生成した UTC 日時で始まるので、タグの順が生成の順になる。
    if not candidates:
        return None
    tag = max(candidates)
    logging.info("前回の正式版 Release を取得しています: %s", tag)
    return fetch(tag, directory / "data", "stable")


def previous_release(directory, channel="stable"):
    if channel == "stable":
        return stable_baseline(directory)
    manifest = read_manifest(channel)
    # 初回公開は、必ず手動確認に回す。
    if manifest is None:
        return None
    tag = release_tag(manifest["version"], channel)
    target = next((r for r in shared.releases() if r["tag_name"] == tag), None)
    if target is None:
        raise shared.MissingPreviousRelease("前回の公開Releaseがありません。削除後の再開は手動公開の初期化を指定してください")
    if target["draft"] or target["prerelease"] != (channel == "dev"):
        raise ValueError("山頂の参照先が公開済みの版ではありません")
    logging.info("前回の山頂公開版を取得しています: %s", tag)
    previous = fetch(tag, directory / "data", channel)
    if previous[0] != manifest:
        raise ValueError("山頂の最新版参照と公開版の manifest が一致しません")
    return previous


def prepare(directory, channel="stable"):
    check_branch(channel)
    logging.info("生成した配布ファイルを検証しています")
    current = validate(directory, channel=channel)
    tag = release_tag(current[0]["version"], channel)
    check_tag(tag, channel)
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        previous = None
        comparison_failed = False
        try:
            previous = previous_release(root / "previous", channel)
            warnings = assess(current, previous)
        except Exception as exc:
            # 比較の失敗で生成物を失わず、公開せずに人の確認へ回す。
            comparison_failed = True
            logging.exception("前回公開版を比較できません")
            warnings = [f"前回公開版の取得・検証・比較に失敗したため、下書きに残します（{type(exc).__name__}: {exc}）"]
        notes = root / "notes.md"
        contents = report(current, previous, warnings).replace("## データの検査結果", "## データの検査結果（生成後）", 1)
        stable = channel == "stable"
        if stable and warnings:
            # Release 一覧で目に入るよう、要確認の理由を本文の先頭に書く。
            contents = f"{shared.REVIEW_HEADING}\n\n" + "".join(f"- {warning}\n" for warning in warnings) + "\n" + contents
        shared.write_report(notes, contents)
        # 正式版も比較が失敗したときは下書きに残す。Pages は更新しない。
        draft = not stable or comparison_failed
        state = ["--draft", "--latest=false"] if draft else ["--latest=false"]
        logging.info("%s を作成し、SVG を内蔵したデータと manifest をアップロードします: %s",
                     "下書き Release" if draft else "Release", tag)
        from scripts.distribution_parts import filenames
        url = shared.gh("release", "create", tag, str(Path(directory) / FILE_NAME), str(Path(directory) / "manifest.json"),
           *[str(Path(directory) / name) for name in filenames("osm-peaks") if (Path(directory) / name).exists()],
           *state, "--target", os.environ["GITHUB_SHA"], "--title", tag,
           "--notes-file", str(notes), f"--prerelease={str(not stable).lower()}").strip()
        if comparison_failed:
            handling = "下書きで保留（前回公開版の比較に失敗。手動確認が必要）"
        elif stable:
            handling = "Release に出しました（要確認。本文の先頭の理由を確認してください）" if warnings else "Release に出しました"
        else:
            handling = "下書きで保留（手動確認が必要）" if warnings else "自動公開の段階へ進みます"
        shared.append_summary(f"\n## 生成したリリース\n\n[生成したリリースを開く]({url})\n\n"
                       f"- タグ: `{tag}`\n"
                       f"- 生成後の扱い: {handling}\n")
    shared.output(tag=tag, sha256=current[0]["sha256"], auto_publish=str(not stable and not warnings).lower())
    logging.info("Release の保存完了: %s（%s）", tag, "要確認" if warnings else "検査合格")


def publish(tag, expected_sha256="", *, manual=False, reason="", channel="stable"):
    check_branch(channel)
    expected_sha256 = expected_sha256.strip()
    if (expected_sha256 or not manual) and not re.fullmatch(r"[0-9a-f]{64}", expected_sha256):
        raise ValueError("確認した gzip の SHA-256 を指定してください")
    release = find_release(tag, channel)
    tag = release["tag_name"]
    if not expected_sha256:
        expected_sha256 = shared.reviewed_checksum(release)
    logging.info("公開対象: %s、照合する SHA-256: %s", tag, expected_sha256)
    if release["prerelease"] != (channel == "dev"):
        raise ValueError("Release の正式版・開発版の区分が一致しません")
    if not release["draft"] and not manual:
        raise ValueError("自動公開の対象は下書きに限ります。公開済みの参照先復旧は手動で行ってください")
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        logging.info("下書きのファイルを取得し、公開前に再検証します: %s", tag)
        current = fetch(tag, root / "current", channel)
        if current[0]["sha256"] != expected_sha256:
            raise ValueError("確認したファイルと下書きの SHA-256 が異なります。公開しません")
        try:
            previous = previous_release(root / "previous", channel)
            warnings = assess(current, previous)
        except shared.MissingPreviousRelease:
            if not manual or os.environ.get("PAGES_INITIALIZE") != "true":
                raise
            previous = None
            warnings = ["前回のReleaseがないため、初期化を指定した手動公開で配布を再開します"]
        except (RuntimeError, ValueError, OSError, KeyError, TypeError, EOFError, zlib.error):
            if release["draft"] or not manual:
                raise
            # 公開済みの検証済みファイルを使い、破損・欠落した参照先だけを復旧する。
            previous = None
            warnings = ["前回の参照先を取得できないため、確認した公開済みの版で参照先を復旧します"]
        contents = report(current, previous, warnings).replace("## データの検査結果", "## データの検査結果（公開前の再検査）", 1)
        if manual:
            contents += f"\n## 手動公開の確認\n\n確認者: {os.environ.get('GITHUB_ACTOR', '不明')}\n\n{reason.strip() or '理由の記入なし'}\n"
        notes = root / "notes.md"
        shared.write_report(notes, contents)
        if release["draft"]:
            shared.gh("release", "edit", tag, "--notes-file", str(notes))
        if warnings and not manual:
            logging.warning("公開直前の検査で要確認になったため、下書きのまま残します")
            return
        logging.info("検証済みの山頂データを公開します: %s", tag)
        if release["draft"]:
            shared.gh("release", "edit", tag, "--draft=false", "--latest=false")
        shared.update_latest(root, current[0], channel)
        shared.append_summary(f"\n## 公開したリリース\n\n"
                       f"[公開したリリースを開く](https://github.com/{os.environ['GH_REPO']}/releases/tag/{tag})\n")
        logging.info("公開完了: %s", tag)


def main():
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(message)s", datefmt="%H:%M:%S")
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    draft = commands.add_parser("prepare")
    draft.add_argument("--channel", choices=("stable", "dev"), default="stable")
    draft.add_argument("--directory", type=Path, default=Path("dist"))
    release = commands.add_parser("publish")
    release.add_argument("--channel", choices=("stable", "dev"), default="stable")
    release.add_argument("--tag", required=True)
    release.add_argument("--sha256", default="")
    release.add_argument("--manual", action="store_true")
    release.add_argument("--reason", default="")
    pages = commands.add_parser("verify-pages")
    pages.add_argument("--directory", type=Path, required=True)
    removal = commands.add_parser("remove-dataset")
    removal.add_argument("--dataset", required=True)
    removal.add_argument("--channel", choices=("stable", "dev"), required=True)
    args = parser.parse_args()
    if args.command == "prepare":
        prepare(args.directory, args.channel)
    elif args.command == "publish":
        publish(args.tag, args.sha256, manual=args.manual, reason=args.reason, channel=args.channel)
    elif args.command == "remove-dataset":
        shared.remove_dataset(args.dataset, args.channel)
    else:
        shared.verify_pages(args.directory)


if __name__ == "__main__":
    main()
