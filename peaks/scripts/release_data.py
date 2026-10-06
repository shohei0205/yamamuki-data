"""下書きの作成と、確認済みの同じファイルの公開を分けて行う。"""

import argparse
from datetime import datetime, timezone
import json
import logging
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
import zlib
import uuid
from urllib.error import HTTPError
from urllib.request import Request, urlopen
from urllib.parse import urlsplit

from scripts.build_data import FILE_NAME
from scripts.check_release import assess, report, validate
from scripts.release_channels import check_branch, check_tag, prefix, release_tag, download_url


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
    matches = [entry for entry in releases()
               if (entry.get("html_url") == target if is_url else entry["tag_name"] == target)]
    if len(matches) != 1:
        raise ValueError("指定した Release を一意に取得できません。URL・タグと読み取り権限を確認してください")
    check_tag(matches[0]["tag_name"], channel)
    return matches[0]


def reviewed_checksum(release):
    """生成時の検査結果に記録した値を使い、手入力の転記を省く。"""
    matches = re.findall(r"^- SHA-256: `([0-9a-f]{64})`$", release.get("body") or "", re.MULTILINE)
    if len(matches) != 1:
        raise ValueError("Release の検査結果から SHA-256 を取得できません。確認した値を明示してください")
    return matches[0]


def manifest_path(channel):
    prefix(channel)
    return ("peaks" if channel == "stable" else "peaks-dev") + "/manifest.json"


def pages_url():
    endpoint("")
    owner, repo = os.environ["GH_REPO"].split("/")
    return f"https://{owner.lower()}.github.io/{repo}"


class Catalog(dict):
    """既存の manifest 一覧の使い方を保ち、公開履歴も同じ応答から引き継ぐ。"""

    def __init__(self, manifests, histories=None):
        super().__init__(manifests)
        self.histories = histories or {}


def history_entry(manifest, channel, *, snapshot=False):
    tag = release_tag(manifest["version"], channel)
    endpoint("")
    repo = os.environ["GH_REPO"]
    run = os.environ.get("GITHUB_RUN_ID")
    attempt = os.environ.get("GITHUB_RUN_ATTEMPT", "1")
    return {
        "kind": "snapshot" if snapshot else "publication",
        "publishedAt": None if snapshot else datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z"),
        "version": manifest["version"],
        "releaseUrl": f"https://github.com/{repo}/releases/tag/{tag}",
        "downloadUrl": manifest.get("downloadUrl") or download_url(manifest["version"], channel),
        "actionsRunUrl": None if snapshot or not run else f"https://github.com/{repo}/actions/runs/{run}/attempts/{attempt}",
    }


def read_catalog():
    # 更新直後の古いキャッシュを避け、サイト全体を1回の応答から引き継ぐ。
    request = Request(f"{pages_url()}/catalog.json?update={uuid.uuid4().hex}",
                      headers={"Cache-Control": "no-cache", "User-Agent": "yamamuki-data"})
    try:
        with urlopen(request, timeout=60) as response:
            catalog = json.load(response)
    except HTTPError as exc:
        if exc.code == 404:
            return None
        raise
    if not isinstance(catalog, dict) or catalog.get("schemaVersion") != 1:
        raise ValueError("配布サイトの一覧の形式が不正です")
    manifests = catalog.get("manifests")
    if not isinstance(manifests, dict):
        raise ValueError("配布サイトの manifest 一覧が不正です")
    for path, manifest in manifests.items():
        if (not re.fullmatch(r"[a-z][a-z0-9-]*/manifest\.json", path)
                or not isinstance(manifest, dict)):
            raise ValueError("配布サイトに不正な manifest のパスや内容があります")
    histories = catalog.get("histories", {})
    if not isinstance(histories, dict):
        raise ValueError("公開履歴の一覧が不正です")
    for path, entries in histories.items():
        if (not re.fullmatch(r"[a-z][a-z0-9-]*/history\.json", path)
                or path.replace("/history.json", "/manifest.json") not in manifests
                or not isinstance(entries, list)
                or not all(isinstance(entry, dict) and isinstance(entry.get("version"), str)
                           and entry.get("kind") in ("snapshot", "publication")
                           and isinstance(entry.get("releaseUrl"), str)
                           and isinstance(entry.get("downloadUrl"), str)
                           and (entry.get("publishedAt") is None or isinstance(entry["publishedAt"], str))
                           and (entry.get("actionsRunUrl") is None or isinstance(entry["actionsRunUrl"], str))
                           for entry in entries)):
            raise ValueError("公開履歴のパスや内容が不正です")
    return Catalog(manifests, histories)


def read_manifest(channel):
    path = manifest_path(channel)
    catalog = read_catalog()
    if catalog is None:
        return None
    return catalog.get(path)


def previous_release(directory, channel="stable"):
    manifest = read_manifest(channel)
    # 初回公開は、必ず手動確認に回す。
    if manifest is None:
        return None
    tag = release_tag(manifest["version"], channel)
    target = next((r for r in releases() if r["tag_name"] == tag), None)
    if target is None or target["draft"] or target["prerelease"] != (channel == "dev"):
        raise ValueError("山頂の参照先が公開済みの版ではありません")
    logging.info("前回の山頂公開版を取得しています: %s", tag)
    previous = fetch(tag, directory / "data", channel)
    if previous[0] != manifest:
        raise ValueError("山頂の最新版参照と公開版の manifest が一致しません")
    return previous


def update_latest(directory, manifest, channel="stable"):
    path = manifest_path(channel)
    release_tag(manifest["version"], channel)
    catalog = read_catalog()
    if catalog is None:
        if os.environ.get("PAGES_INITIALIZE") != "true":
            raise ValueError("配布サイトの一覧を取得できません。初回公開だけ手動公開の初期化を指定してください")
        catalog = {}
    histories = {key: list(entries) for key, entries in getattr(catalog, "histories", {}).items()}
    # 履歴機能の導入前の最新版も残す。分からない公開日時や実行 URL は補わない。
    for previous_channel in ("stable", "dev"):
        previous_path = manifest_path(previous_channel)
        history_path = previous_path.replace("manifest.json", "history.json")
        if previous_path in catalog and history_path not in histories:
            histories[history_path] = [history_entry(catalog[previous_path], previous_channel, snapshot=True)]
    history_path = path.replace("manifest.json", "history.json")
    histories.setdefault(history_path, []).append(history_entry(manifest, channel))
    catalog[path] = manifest
    destination = Path(os.environ.get("PAGES_DIRECTORY", "../build/pages"))
    if destination.exists():
        raise ValueError("Pages の出力先が既にあります。空の出力先を指定してください")
    destination.mkdir(parents=True)
    for target, contents in catalog.items():
        file = destination / target
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(json.dumps(contents, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    for target, entries in histories.items():
        file = destination / target
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(json.dumps({"schemaVersion": 1, "entries": entries}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")
    (destination / "catalog.json").write_text(
        json.dumps({"schemaVersion": 1, "manifests": catalog, "histories": histories}, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8", newline="\n")
    (destination / ".nojekyll").write_text("", encoding="utf-8")
    # Actions の Pages 配置が成功するまで「更新完了」とは扱わない。
    output(pages_ready="true")
    append_summary(f"\n## 配置する最新版の参照先\n\n[manifest.json を開く]({pages_url()}/{path})\n"
                   "\nPages の配置結果は後続のステップで確認してください。\n")


def verify_pages(directory):
    expected = json.loads((directory / "catalog.json").read_text(encoding="utf-8"))
    # 配置直後の反映を待ってから、次の公開ジョブに進ませる。
    for attempt in range(12):
        try:
            actual = read_catalog()
            if (actual == expected["manifests"]
                    and getattr(actual, "histories", {}) == expected.get("histories", {})):
                append_summary("\n## Pages の配置結果\n\n成功: 公開先の manifest 一覧が配置内容と一致しました。\n")
                return
        except (OSError, ValueError):
            logging.warning("Pages の反映をまだ確認できません", exc_info=True)
        if attempt < 11:
            time.sleep(5)
    raise RuntimeError("Pages の配置内容を取得できません。次の公開前に配布サイトを確認してください")


def write_report(path, contents):
    path.write_text(contents, encoding="utf-8-sig", newline="\n")
    append_summary(contents)


def append_summary(contents):
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
    current = validate(directory, channel=channel)
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
        write_report(notes, report(current, previous, warnings).replace("## データの検査結果", "## データの検査結果（生成後）", 1))
        logging.info("下書き Release を作成し、2ファイルをアップロードします: %s", tag)
        url = gh("release", "create", tag, str(Path(directory) / FILE_NAME), str(Path(directory) / "manifest.json"),
           "--draft", "--target", os.environ["GITHUB_SHA"], "--title", f"全国の山データ ({channel}) {current[0]['version']}",
           "--notes-file", str(notes), f"--prerelease={str(channel == 'dev').lower()}").strip()
        append_summary(f"\n## 生成したリリース\n\n[生成したリリースを開く]({url})\n\n"
                       f"- タグ: `{tag}`\n"
                       f"- 生成後の扱い: {'下書きで保留（手動確認が必要）' if warnings else '自動公開の段階へ進みます'}\n")
    output(tag=tag, sha256=current[0]["sha256"], auto_publish=str(not warnings).lower())
    logging.info("下書きの保存完了: %s（%s）", tag, "要確認・自動公開しません" if warnings else "検査合格・公開段階へ進みます")


def publish(tag, expected_sha256="", *, manual=False, reason="", channel="stable"):
    check_branch(channel)
    expected_sha256 = expected_sha256.strip()
    if (expected_sha256 or not manual) and not re.fullmatch(r"[0-9a-f]{64}", expected_sha256):
        raise ValueError("確認した gzip の SHA-256 を指定してください")
    release = find_release(tag, channel)
    tag = release["tag_name"]
    if not expected_sha256:
        expected_sha256 = reviewed_checksum(release)
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
        append_summary(f"\n## 公開したリリース\n\n"
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
    args = parser.parse_args()
    if args.command == "prepare":
        prepare(args.directory, args.channel)
    elif args.command == "publish":
        publish(args.tag, args.sha256, manual=args.manual, reason=args.reason, channel=args.channel)
    else:
        verify_pages(args.directory)


if __name__ == "__main__":
    main()
