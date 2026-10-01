"""下書きの作成と、確認済みの同じファイルの公開を分けて行う。"""

import argparse
import json
import logging
import os
from pathlib import Path
import re
import subprocess
import tempfile
import zlib

from scripts.build_data import FILE_NAME
from scripts.check_release import assess, report, validate
from scripts.release_channels import check_branch, check_tag, prefix, release_tag


def gh(*arguments):
    result = subprocess.run(["gh", *arguments], capture_output=True, text=True, encoding="utf-8")
    if result.returncode:
        raise RuntimeError(result.stderr.strip() or "GitHub CLI が失敗しました")
    return result.stdout


def endpoint(suffix):
    repo = os.environ["GH_REPO"]
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo):
        raise ValueError("リポジトリ名が不正です")
    return f"repos/{repo}/{suffix}"


def fetch(tag, directory, channel="stable"):
    check_tag(tag, channel)
    gh("release", "download", tag, "--pattern", "manifest.json", "--pattern", FILE_NAME, "--dir", str(directory))
    return validate(directory, tag=tag, channel=channel)


def releases():
    # 通信失敗や権限不足を「初回」と扱わない。
    pages = json.loads(gh("api", "--paginate", "--slurp", endpoint("releases?per_page=100")))
    return [release for page in pages for release in page]


def previous_release(directory, channel="stable"):
    latest_tag = prefix(channel) + "latest"
    entries = releases()
    latest = next((r for r in entries if r["tag_name"] == latest_tag), None)
    # 初回公開や初回の参照先作成失敗は、必ず手動確認に回す。
    if latest is None or latest["draft"]:
        return None
    if latest["prerelease"] != (channel == "dev"):
        raise ValueError("最新版参照の正式版・開発版の区分が一致しません")
    pointer = directory / "pointer"
    gh("release", "download", latest_tag, "--pattern", "manifest.json", "--dir", str(pointer))
    manifest = json.loads((pointer / "manifest.json").read_text(encoding="utf-8"))
    tag = release_tag(manifest["version"], channel)
    check_tag(tag, channel)
    target = next((r for r in entries if r["tag_name"] == tag), None)
    if target is None or target["draft"] or target["prerelease"] != (channel == "dev"):
        raise ValueError("山頂の参照先が公開済みの版ではありません")
    logging.info("前回の山頂公開版を取得しています: %s", tag)
    previous = fetch(tag, directory / "data", channel)
    if previous[0] != manifest:
        raise ValueError("山頂の最新版参照と公開版の manifest が一致しません")
    return previous


def update_latest(directory, manifest, channel="stable"):
    latest_tag = prefix(channel) + "latest"
    # 検証済みの履歴版が公開された後だけ、山頂専用の参照先を更新する。
    latest = next((r for r in releases() if r["tag_name"] == latest_tag), None)
    if latest is not None and latest["prerelease"] != (channel == "dev"):
        raise ValueError("最新版参照の正式版・開発版の区分が一致しません")
    path = directory / "manifest.json"
    path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    if latest is None:
        gh("release", "create", latest_tag, str(path), "--draft", "--target", os.environ["GITHUB_SHA"],
           "--title", f"山頂データの最新版参照 ({channel})", "--notes",
           f"manifest.json の version に対応する {prefix(channel)}<version> のデータを取得してください。",
           f"--prerelease={str(channel == 'dev').lower()}")
    else:
        gh("release", "upload", latest_tag, str(path), "--clobber")
    gh("release", "edit", latest_tag, "--draft=false", "--latest=false",
       f"--prerelease={str(channel == 'dev').lower()}")


def write_report(path, contents):
    path.write_text(contents, encoding="utf-8-sig", newline="\n")
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as stream:
            stream.write(contents)


def output(**values):
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
            for key, value in values.items():
                stream.write(f"{key}={value}\n")


def prepare(directory, channel="stable"):
    check_branch(channel)
    logging.info("生成した配布ファイルを検証しています")
    current = validate(directory)
    tag = release_tag(current[0]["version"], channel)
    check_tag(tag, channel)
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        previous = None
        try:
            previous = previous_release(root / "previous", channel)
            warnings = assess(current, previous)
        except (RuntimeError, ValueError, OSError, KeyError, TypeError, EOFError, zlib.error) as exc:
            logging.warning("前回公開版を比較できません: %s", exc)
            warnings = ["前回公開版の取得・検証に失敗したため、自動公開しません。Actions のログを確認してください"]
        notes = root / "notes.md"
        write_report(notes, report(current, previous, warnings))
        logging.info("下書き Release を作成し、2ファイルをアップロードします: %s", tag)
        gh("release", "create", tag, str(Path(directory) / FILE_NAME), str(Path(directory) / "manifest.json"),
           "--draft", "--target", os.environ["GITHUB_SHA"], "--title", f"全国の山データ ({channel}) {current[0]['version']}",
           "--notes-file", str(notes), f"--prerelease={str(channel == 'dev').lower()}")
    output(tag=tag, sha256=current[0]["sha256"], auto_publish=str(not warnings).lower())
    logging.info("下書きの保存完了: %s（%s）", tag, "要確認・自動公開しません" if warnings else "検査合格・公開段階へ進みます")


def publish(tag, expected_sha256, *, manual=False, reason="", channel="stable"):
    check_branch(channel)
    check_tag(tag, channel)
    if not re.fullmatch(r"[0-9a-f]{64}", expected_sha256):
        raise ValueError("確認した gzip の SHA-256 を指定してください")
    if manual and not reason.strip():
        raise ValueError("確認内容・公開理由を指定してください")
    release = json.loads(gh("api", endpoint(f"releases/tags/{tag}")))
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
        except (RuntimeError, ValueError, OSError, KeyError, TypeError, EOFError, zlib.error):
            if release["draft"] or not manual:
                raise
            # 公開済みの検証済みファイルを使い、破損・欠落した参照先だけを復旧する。
            previous = None
            warnings = ["前回の参照先を取得できないため、確認した公開済みの版で参照先を復旧します"]
        contents = report(current, previous, warnings)
        if manual:
            contents += f"\n## 手動公開の確認\n\n確認者: {os.environ.get('GITHUB_ACTOR', '不明')}\n\n{reason.strip()}\n"
        notes = root / "notes.md"
        write_report(notes, contents)
        if release["draft"]:
            gh("release", "edit", tag, "--notes-file", str(notes))
        if warnings and not manual:
            logging.warning("公開直前の検査で要確認になったため、下書きのまま残します")
            return
        logging.info("検証済みの山頂データを公開します: %s", tag)
        if release["draft"]:
            gh("release", "edit", tag, "--draft=false", "--latest=false")
        update_latest(root, current[0], channel)
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
    release.add_argument("--sha256", required=True)
    release.add_argument("--manual", action="store_true")
    release.add_argument("--reason", default="")
    args = parser.parse_args()
    if args.command == "prepare":
        prepare(args.directory, args.channel)
    else:
        publish(args.tag, args.sha256, manual=args.manual, reason=args.reason, channel=args.channel)


if __name__ == "__main__":
    main()
