"""公開前の山頂 Draft について、参照先を確認してから整理する。"""

import argparse
import base64
from datetime import datetime, timedelta, timezone
import hashlib
import json
import os
import re
from urllib.parse import unquote, urlsplit
from urllib.request import Request, urlopen
from uuid import uuid4

from release_tools import release_data as site


DATA_REPO = "shohei0205/yamamuki-data"
APP_REPO = "shohei0205/yamamuki"
APP_PAGES = "https://shohei0205.github.io/yamamuki/data/"
TAG = re.compile(r"(?:osm-peaks|gsi-peaks)-\d{8}T\d{6}Z-\d+-\d+")
JST = timezone(timedelta(hours=9))
RETENTION = timedelta(days=7)


def draft_date(release, now):
    """作成から 7 日以上経った公開前の Draft なら作成日を返す。"""
    if (release.get("draft") is not True or "published_at" not in release
            or release["published_at"] is not None
            or site.TEMPORARY_DRAFT_MARKER not in (release.get("body") or "").splitlines()
            or not TAG.fullmatch(release.get("tag_name", ""))):
        return None
    try:
        created = datetime.fromisoformat(release["created_at"])
        if created.tzinfo is None:
            return None
    except (KeyError, TypeError, ValueError):
        return None
    if now - created >= RETENTION:
        return created.astimezone(JST).date()
    return None

def strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, list):
        for item in value:
            yield from strings(item)
    elif isinstance(value, dict):
        for item in value.values():
            yield from strings(item)


def referenced_tags(*documents):
    for document in documents:
        for value in strings(document):
            url = urlsplit(value) if value.startswith("https://") else None
            if url is None or url.netloc.lower() != "github.com":
                continue
            parts = unquote(url.path).strip("/").split("/")
            if (len(parts) >= 5 and "/".join(parts[:2]).lower() == DATA_REPO
                    and parts[2] == "releases" and parts[3] in ("tag", "download")):
                yield parts[4]


def read_json_url(url, expected_sha256=None):
    request = Request(f"{url}?cleanup={uuid4().hex}",
                      headers={"Cache-Control": "no-cache", "User-Agent": "yamamuki-data"})
    with urlopen(request, timeout=30) as response:
        raw = response.read()
    if expected_sha256 is not None and hashlib.sha256(raw).hexdigest() != expected_sha256:
        raise ValueError(f"公開 manifest の SHA-256 がカタログと一致しません: {url}")
    document = json.loads(raw)
    if not isinstance(document, dict):
        raise ValueError(f"JSON の内容が不正です: {url}")
    return document


def app_revision():
    return json.loads(site.gh("api", f"repos/{APP_REPO}/commits/main"))


def app_source_documents(revision):
    tree_sha = revision["commit"]["tree"]["sha"]
    tree = json.loads(site.gh("api", f"repos/{APP_REPO}/git/trees/{tree_sha}?recursive=1"))
    if tree.get("truncated") is not False:
        raise ValueError("アプリ側のファイル一覧を最後まで取得できません")
    paths = {entry["path"]: entry for entry in tree["tree"]
             if entry["path"].startswith("site/data/") and entry["path"].endswith(".json")}
    if "site/data/catalog.json" not in paths:
        raise ValueError("アプリ側 main にカタログがありません")
    for path, entry in paths.items():
        if entry["type"] != "blob" or entry["mode"] not in ("100644", "100755"):
            raise ValueError(f"参照ファイルの種類が不正です: {path}")
        blob = json.loads(site.gh("api", f"repos/{APP_REPO}/git/blobs/{entry['sha']}"))
        if blob.get("encoding") != "base64":
            raise ValueError(f"参照ファイルを取得できません: {path}")
        document = json.loads(base64.b64decode(blob["content"]))
        if not isinstance(document, dict) or ("/manifests/" in path
                                              and not isinstance(document.get("downloadUrl"), str)):
            raise ValueError(f"参照ファイルの形式が不正です: {path}")
        yield document


def app_pointers(catalog, legacy):
    if (catalog.get("schemaVersion") != 1 or not isinstance(catalog.get("datasets"), dict)
            or "osm-peaks" not in catalog["datasets"] or legacy.get("schemaVersion") != 1
            or "stable" not in legacy):
        raise ValueError("アプリの公開カタログの形式が不正です")
    osm = catalog["datasets"]["osm-peaks"]
    if not isinstance(osm, dict) or "stable" not in osm:
        raise ValueError("アプリの公開カタログに OSM 山頂の正式版がありません")
    for channels in (*catalog["datasets"].values(), legacy):
        if not isinstance(channels, dict):
            raise ValueError("アプリの配布先の形式が不正です")
        for channel in ("stable", "dev"):
            if channel not in channels:
                continue
            pointer = channels[channel]
            url = pointer.get("manifestUrl") if isinstance(pointer, dict) else None
            if not isinstance(url, str) or not re.fullmatch(
                    re.escape(APP_PAGES) + r"[a-z0-9_-]+/manifests/[a-zA-Z0-9_.-]+\.json", url):
                raise ValueError("アプリの manifest の参照先が不正です")
            digest = pointer.get("manifestSha256")
            if not isinstance(digest, str) or not re.fullmatch(r"[a-f0-9]{64}", digest):
                raise ValueError("アプリの manifest の SHA-256 が不正です")
            yield url, digest


def data_pointers(catalog, channel):
    if catalog.get("schemaVersion") != 2 or not isinstance(catalog.get("datasets"), list):
        raise ValueError("データ側の地点カタログの形式が不正です")
    prefix = site.pages_url()
    suffix = "-dev" if channel == "dev" else ""
    for entry in catalog["datasets"]:
        dataset = entry["id"]
        url = f"{prefix}/points/{dataset}{suffix}/manifest.json"
        if (not re.fullmatch(r"[a-z][a-z0-9_-]*", dataset) or entry.get("manifestUrl") != url
                or not isinstance(entry.get("manifest"), dict)):
            raise ValueError("データ側の manifest の参照先が不正です")
        yield url


def collect_references():
    revision = app_revision()
    data = site.read_catalog()
    if data is None:
        raise ValueError("データ側の公開一覧を取得できません")
    app = read_json_url(APP_PAGES + "catalog.json")
    legacy = read_json_url(APP_PAGES + "osm-peaks/current.json")
    protected = set(referenced_tags(data, data.histories, app, legacy, *app_source_documents(revision)))
    app_manifests = {}
    for url, digest in app_pointers(app, legacy):
        if app_manifests.setdefault(url, digest) != digest:
            raise ValueError(f"アプリのカタログ間で manifest の SHA-256 が異なります: {url}")
    urls = set(app_manifests)
    # カタログを経由せず直接読む既存アプリの参照先も確認する。
    urls.add(f"{site.pages_url()}/points/osm-peaks-dev/manifest.json")
    for channel in ("stable", "dev"):
        catalog = site.read_points_catalog(channel)
        urls.update(data_pointers(catalog, channel))
        protected.update(referenced_tags(catalog))
    for url in sorted(urls):
        document = read_json_url(url, app_manifests.get(url))
        if type(document.get("schemaVersion")) is not int or not isinstance(document.get("downloadUrl"), str):
            raise ValueError(f"公開 manifest の形式が不正です: {url}")
        protected.update(referenced_tags(document))
    return protected, revision["sha"]


def list_candidates(now):
    eligible = [(created, release["tag_name"]) for release in site.releases()
                if (created := draft_date(release, now)) is not None]
    protected, _ = collect_references()
    candidates = [(created, tag) for created, tag in sorted(eligible) if tag not in protected]
    lines = ["## Draft の削除候補", ""]
    lines += [f"- `{tag}`（作成日 {created}）" for created, tag in candidates] or ["候補はありません。"]
    site.append_summary("\n".join(lines) + "\n")


def delete_candidate(tag, now):
    if (os.environ.get("GITHUB_ACTIONS") != "true" or os.environ.get("GITHUB_REF") != "refs/heads/main"
            or os.environ.get("GH_REPO") != DATA_REPO):
        raise ValueError("削除はデータリポジトリの main の Action から実行してください")
    if not TAG.fullmatch(tag):
        raise ValueError("山頂データの Draft タグを 1 件指定してください")
    matches = [release for release in site.releases() if release.get("tag_name") == tag]
    if len(matches) != 1 or draft_date(matches[0], now) is None:
        raise ValueError("指定した Draft は削除条件を満たしていません")
    release_id = matches[0]["id"]
    if type(release_id) is not int or release_id <= 0:
        raise ValueError("Release ID が不正です")
    protected, app_sha = collect_references()
    if tag in protected:
        raise ValueError("指定した版を参照しているカタログまたは manifest があります")
    if app_revision()["sha"] != app_sha:
        raise ValueError("確認中にアプリ側 main が変わりました。再実行してください")
    release = json.loads(site.gh("api", site.endpoint(f"releases/{release_id}")))
    if release.get("id") != release_id or release.get("tag_name") != tag or draft_date(release, now) is None:
        raise ValueError("指定した Release は削除条件を満たしていません")
    site.gh("api", "--method", "DELETE", site.endpoint(f"releases/{release_id}"))
    site.append_summary(f"## Draft の削除\n\n- 削除した Release: `{tag}`\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("list")
    commands.add_parser("delete-old")
    delete = commands.add_parser("delete")
    delete.add_argument("--tag", required=True)
    args = parser.parse_args()
    now = datetime.now(timezone.utc)
    if args.command == "list":
        list_candidates(now)
    elif args.command == "delete":
        delete_candidate(args.tag, now)
    else:
        tags = [release["tag_name"] for release in site.releases() if draft_date(release, now) is not None]
        for tag in tags:
            delete_candidate(tag, now)
        if not tags:
            site.append_summary("## Draft の整理\n\n対象はありません。\n")


if __name__ == "__main__":
    main()
