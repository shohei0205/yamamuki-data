"""確認済みの Release をデータ種別ごとの公開処理に渡す共通入口。"""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
from urllib.parse import urlsplit


# 地形の検証・公開処理ができたら、ここに登録する。未対応の種別は公開しない。
PUBLISHERS = {"peaks": ("peaks", "scripts.release_data")}
ROOT = Path(__file__).resolve().parents[1]


def dataset_for_tag(tag, channel):
    if channel not in ("stable", "dev"):
        raise ValueError("配布先は stable または dev を指定してください")
    for dataset in PUBLISHERS:
        prefix = dataset + ("-dev-" if channel == "dev" else "-")
        if tag.startswith(prefix):
            version = tag[len(prefix):]
            if (re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", version)
                    and version != "latest" and not version.startswith("dev-")):
                return dataset
    raise ValueError("未対応のデータ種別、最新版参照タグ、または配布先の異なるタグです")


def check_branch(channel):
    if os.environ.get("GITHUB_ACTIONS") == "true":
        expected = {"refs/heads/dev": "dev", "refs/heads/main": "stable"}.get(os.environ.get("GITHUB_REF"))
        if expected != channel:
            raise ValueError("正式版は main、開発版は dev から実行してください")


def resolve(target, channel):
    check_branch(channel)
    repo = os.environ["GH_REPO"]
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo):
        raise ValueError("リポジトリ名が不正です")
    target = target.strip()
    is_url = target.startswith("https://")
    if is_url:
        parsed = urlsplit(target)
        start = f"/{repo}/releases/tag/"
        if (parsed.netloc != "github.com" or parsed.query or parsed.fragment
                or not parsed.path.startswith(start)
                or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", parsed.path[len(start):])):
            raise ValueError("このリポジトリの Release ページの URL を指定してください")
    else:
        dataset_for_tag(target, channel)
    result = subprocess.run(
        ["gh", "api", "--paginate", "--slurp", f"repos/{repo}/releases?per_page=100"],
        check=True, capture_output=True, text=True, encoding="utf-8")
    entries = [entry for page in json.loads(result.stdout) for entry in page]
    matches = [entry for entry in entries
               if (entry.get("html_url") == target if is_url else entry["tag_name"] == target)]
    if len(matches) != 1:
        raise ValueError("指定した Release を一意に取得できません")
    release = matches[0]
    dataset = dataset_for_tag(release["tag_name"], channel)
    if release["prerelease"] != (channel == "dev"):
        raise ValueError("Release の正式版・開発版の区分が一致しません")
    return dataset, release["tag_name"]


def publish(dataset, tag, channel, sha256, reason):
    check_branch(channel)
    if dataset_for_tag(tag, channel) != dataset:
        raise ValueError("データ種別とタグが一致しません")
    directory, module = PUBLISHERS[dataset]
    # コマンドと作業場所は登録済みの値だけを使う。検証・公開は各種別が担当する。
    subprocess.run([sys.executable, "-m", module, "publish", "--channel", channel,
                    "--tag", tag, "--sha256", sha256, "--manual", "--reason", reason],
                   cwd=ROOT / directory, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("resolve", "publish"))
    parser.add_argument("--channel", choices=("stable", "dev"), required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--dataset")
    parser.add_argument("--sha256", default="")
    parser.add_argument("--reason", default="")
    args = parser.parse_args()
    if args.command == "resolve":
        dataset, tag = resolve(args.tag, args.channel)
        print(f"公開対象: {dataset} / {args.channel} / {tag}")
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
            stream.write(f"dataset={dataset}\ntag={tag}\n")
    else:
        publish(args.dataset, args.tag, args.channel, args.sha256, args.reason)


if __name__ == "__main__":
    main()
