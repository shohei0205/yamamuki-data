"""Release の取得と、データ共通の Pages・公開履歴の更新を行う。"""

import argparse
from datetime import datetime, timezone
import json
import logging
import os
from pathlib import Path
import re
import subprocess
import time
import uuid
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from release_tools.publish_reviewed import ROOT, check_branch, check_dataset, version_for_tag

REVIEW_HEADING = "## 要確認"


def check_channel(channel):
    if channel not in ("stable", "dev"):
        raise ValueError("配布先は stable または dev を指定してください")


def release_tag(version, channel="stable", *, dataset="osm-peaks"):
    tag = dataset + ("-dev-" if channel == "dev" else "-") + version
    version_for_tag(dataset, tag, channel)
    return tag


def manifest_path(channel, *, dataset="osm-peaks"):
    check_dataset(dataset)
    check_channel(channel)
    return f"points/{dataset}{'-dev' if channel == 'dev' else ''}/manifest.json"


def download_url(version, channel="stable", *, dataset="osm-peaks"):
    endpoint("")
    return f"https://github.com/{os.environ['GH_REPO']}/releases/download/{release_tag(version, channel, dataset=dataset)}/{dataset}.json.gz"


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


def releases():
    # 通信失敗や権限不足を「初回」と扱わない。
    pages = json.loads(gh("api", "--paginate", "--slurp", endpoint("releases?per_page=100")))
    return [release for page in pages for release in page]


def reviewed_checksum(release):
    """生成時の検査結果に記録した値を使い、手入力の転記を省く。"""
    matches = re.findall(r"^- SHA-256: `([0-9a-f]{64})`$", release.get("body") or "", re.MULTILINE)
    if len(matches) != 1:
        raise ValueError("Release の検査結果から SHA-256 を取得できません。確認した値を明示してください")
    return matches[0]


def pages_url():
    endpoint("")
    owner, repo = os.environ["GH_REPO"].split("/")
    return f"https://{owner.lower()}.github.io/{repo}"


class Catalog(dict):
    """既存の manifest 一覧の使い方を保ち、公開履歴も同じ応答から引き継ぐ。"""

    def __init__(self, manifests, histories=None):
        super().__init__(manifests)
        self.histories = histories or {}


def history_entry(manifest, channel, *, snapshot=False, dataset=None):
    tag = release_tag(manifest["version"], channel, dataset=dataset or "osm-peaks")
    endpoint("")
    repo = os.environ["GH_REPO"]
    run = os.environ.get("GITHUB_RUN_ID")
    attempt = os.environ.get("GITHUB_RUN_ATTEMPT", "1")
    return {
        "kind": "snapshot" if snapshot else "publication",
        "publishedAt": None if snapshot else datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z"),
        "version": manifest["version"],
        "releaseUrl": f"https://github.com/{repo}/releases/tag/{tag}",
        "downloadUrl": manifest.get("downloadUrl") or download_url(manifest["version"], channel, dataset=dataset or "osm-peaks"),
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
        if (not re.fullmatch(r"(?:[a-z][a-z0-9_-]*/)+manifest\.json", path)
                or not isinstance(manifest, dict)):
            raise ValueError("配布サイトに不正な manifest のパスや内容があります")
    histories = catalog.get("histories", {})
    if not isinstance(histories, dict):
        raise ValueError("公開履歴の一覧が不正です")
    for path, entries in histories.items():
        if (not re.fullmatch(r"(?:[a-z][a-z0-9_-]*/)+history\.json", path)
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


class MissingPreviousRelease(ValueError):
    """公開サイトが指すReleaseが削除されている。"""


def needs_review(release):
    return (release.get("body") or "").lstrip("\ufeff").startswith(REVIEW_HEADING)


def points_catalog(manifests, selected_channel="stable"):
    """公開済み地点データの取得先とダウンロード検証情報をまとめる。"""
    check_channel(selected_channel)
    datasets = []
    for path in sorted(manifests):
        match = re.fullmatch(r"points/([a-z][a-z0-9_-]*)/manifest\.json", path)
        if match:
            name = match[1]
            channel = "dev" if name.endswith("-dev") else "stable"
            if channel != selected_channel:
                continue
            dataset_id = name[:-4] if channel == "dev" else name
            manifest = dict(manifests[path])
            # 従来の山頂は manifest の版で本体の形式も管理していた。
            if (dataset_id == "osm-peaks" and "dataSchemaVersion" not in manifest
                    and type(manifest.get("schemaVersion")) is int and manifest["schemaVersion"] in (1, 2, 3, 4, 5, 6)):
                manifest["dataSchemaVersion"] = manifest["schemaVersion"]
            # 版1〜3の山頂 manifest にも、本体を直接取得できる URL を補う。
            if dataset_id == "osm-peaks" and "downloadUrl" not in manifest and "version" in manifest:
                manifest["downloadUrl"] = download_url(manifest["version"], channel)
            display_name = manifest.get("name", "山頂" if dataset_id == "osm-peaks" else dataset_id)
            if not isinstance(display_name, str) or not display_name.strip():
                raise ValueError("データセットの表示名が不正です")
            datasets.append({"id": dataset_id, "name": display_name.strip(),
                             "manifestUrl": f"{pages_url()}/{path}", "manifest": manifest})
    return {"schemaVersion": 2, "datasets": datasets}


def read_points_catalog(channel="stable"):
    check_channel(channel)
    filename = "catalog.json" if channel == "stable" else "catalog-dev.json"
    request = Request(f"{pages_url()}/points/{filename}?update={uuid.uuid4().hex}",
                      headers={"Cache-Control": "no-cache", "User-Agent": "yamamuki-data"})
    with urlopen(request, timeout=60) as response:
        return json.load(response)


def update_latest(directory, manifest, channel="stable", *, dataset=None):
    selected_dataset = dataset or "osm-peaks"
    release_tag(manifest["version"], channel, dataset=selected_dataset)
    def target_path(target_channel):
        return manifest_path(target_channel, dataset=selected_dataset)
    path = target_path(channel)
    catalog = read_catalog()
    if catalog is None:
        if os.environ.get("PAGES_INITIALIZE") != "true":
            raise ValueError("配布サイトの一覧を取得できません。初回公開だけ手動公開の初期化を指定してください")
        catalog = {}
    histories = {key: list(entries) for key, entries in getattr(catalog, "histories", {}).items()}
    # 履歴機能の導入前の最新版も残す。分からない公開日時や実行 URL は補わない。
    for previous_channel in ("stable", "dev"):
        previous_path = target_path(previous_channel)
        history_path = previous_path.replace("manifest.json", "history.json")
        if previous_path in catalog and history_path not in histories:
            histories[history_path] = [history_entry(catalog[previous_path], previous_channel, snapshot=True, **({"dataset": dataset} if dataset else {}))]
    history_path = path.replace("manifest.json", "history.json")
    histories.setdefault(history_path, []).append(history_entry(manifest, channel, **({"dataset": dataset} if dataset else {})))
    catalog[path] = manifest
    write_site(catalog, histories)
    # Actions の Pages 配置が成功するまで「更新完了」とは扱わない。
    output(pages_ready="true")
    append_summary(f"\n## 配置する最新版の参照先\n\n[manifest.json を開く]({pages_url()}/{path})\n"
                   "\nPages の配置結果は後続のステップで確認してください。\n")


def write_site(catalog, histories):
    """公開・削除で共通のサイト一式を生成する。"""
    destination = Path(os.environ.get("PAGES_DIRECTORY", ROOT / "build/pages"))
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
    points_directory = destination / "points"
    points_directory.mkdir(exist_ok=True)
    for channel, filename in (("stable", "catalog.json"), ("dev", "catalog-dev.json")):
        (points_directory / filename).write_text(
            json.dumps(points_catalog(catalog, channel), ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8", newline="\n")
    (destination / ".nojekyll").write_text("", encoding="utf-8")


def remove_dataset(dataset, channel):
    check_branch(channel)
    if (not re.fullmatch(r"[a-z][a-z0-9_-]*", dataset) or dataset.endswith("-dev")):
        raise ValueError("正式版のデータセット名を指定してください")
    check_channel(channel)
    path = f"points/{dataset}{'-dev' if channel == 'dev' else ''}/manifest.json"
    catalog = read_catalog()
    if catalog is None:
        raise ValueError("既存のカタログを取得できないため削除を中止します")
    targets = [path]
    if not any(target in catalog for target in targets):
        raise ValueError("指定したデータセット・配布先はカタログにありません")
    manifests = dict(catalog)
    histories = dict(getattr(catalog, "histories", {}))
    for target in targets:
        manifests.pop(target, None)
        histories.pop(target.replace("manifest.json", "history.json"), None)
    write_site(manifests, histories)
    output(pages_ready="true")
    append_summary(f"\n## カタログから削除\n\n- データセット: `{dataset}`\n- 配布先: `{channel}`\n"
                   "- Release とデータ本体は保持します。Pages の配置後に削除が反映されます。\n")


def verify_pages(directory):
    expected = json.loads((directory / "catalog.json").read_text(encoding="utf-8"))
    expected_points = {}
    for channel, filename in (("stable", "catalog.json"), ("dev", "catalog-dev.json")):
        points_file = directory / "points" / filename
        if points_file.exists():
            expected_points[channel] = json.loads(points_file.read_text(encoding="utf-8"))
    # 配置直後の反映を待ってから、次の公開ジョブに進ませる。
    for attempt in range(12):
        try:
            actual = read_catalog()
            if (actual == expected["manifests"]
                    and getattr(actual, "histories", {}) == expected.get("histories", {})
                    and all(read_points_catalog(channel) == contents for channel, contents in expected_points.items())):
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    pages = commands.add_parser("verify-pages")
    pages.add_argument("--directory", type=Path, required=True)
    removal = commands.add_parser("remove-dataset")
    removal.add_argument("--dataset", required=True)
    removal.add_argument("--channel", choices=("stable", "dev"), required=True)
    args = parser.parse_args()
    if args.command == "remove-dataset":
        remove_dataset(args.dataset, args.channel)
    else:
        verify_pages(args.directory)


if __name__ == "__main__":
    main()
