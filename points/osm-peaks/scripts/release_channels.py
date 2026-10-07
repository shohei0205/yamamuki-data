"""山頂データの正式版と開発版の参照先を分ける。"""

import os
import re


def prefix(channel):
    if channel not in ("stable", "dev"):
        raise ValueError("配布先は stable または dev を指定してください")
    return "osm-peaks-" if channel == "stable" else "osm-peaks-dev-"


def release_tag(version, channel="stable"):
    if not isinstance(version, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", version):
        raise ValueError("版の形式が不正です")
    # 固定の参照タグや開発版の名前空間との衝突を防ぐ。
    if version == "latest" or version.startswith("dev-"):
        raise ValueError("latest と dev- で始まる版名は予約されています")
    return prefix(channel) + version


def check_tag(tag, channel="stable"):
    start = prefix(channel)
    if not tag.startswith(start) or release_tag(tag[len(start):], channel) != tag:
        raise ValueError("タグと配布先が一致しません")


def check_branch(channel):
    """Actions では実行元ブランチと異なる配布先への書き込みを止める。"""
    prefix(channel)
    if os.environ.get("GITHUB_ACTIONS") != "true":
        return
    expected = {"refs/heads/main": "stable", "refs/heads/dev": "dev"}.get(os.environ.get("GITHUB_REF"))
    if expected != channel:
        raise ValueError("正式版は main、開発版は dev ブランチから実行してください")


def download_url(version, channel="stable"):
    repo = os.environ.get("GH_REPO") or os.environ.get("GITHUB_REPOSITORY", "shohei0205/yamamuki-data")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo):
        raise ValueError("リポジトリ名が不正です")
    return f"https://github.com/{repo}/releases/download/{release_tag(version, channel)}/osm-peaks.json.gz"
