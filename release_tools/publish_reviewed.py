"""確認済みの Release をデータ種別ごとの公開処理に渡す共通入口。"""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
from urllib.parse import urlsplit


ROOT = Path(__file__).resolve().parents[1]


def check_dataset(dataset):
    if (not isinstance(dataset, str) or not re.fullmatch(r"[a-z][a-z0-9_-]*", dataset)
            or dataset.endswith("-dev")):
        raise ValueError("データ ID が不正です")


def version_for_tag(dataset, tag, channel):
    check_dataset(dataset)
    if channel not in ("stable", "dev"):
        raise ValueError("配布先は stable または dev を指定してください")
    prefix = dataset + ("-dev-" if channel == "dev" else "-")
    if isinstance(tag, str) and tag.startswith(prefix):
        version = tag[len(prefix):]
        if (re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", version)
                and version != "latest" and not version.startswith("dev-")):
            return version
    raise ValueError("データ ID・版・配布先とタグが一致しません")


def dataset_for_tag(tag, channel, version):
    # 版名にもハイフンを使えるため、タグだけを区切って推測しない。
    if not isinstance(version, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", version):
        raise ValueError("manifest の版が不正です")
    suffix = ("-dev-" if channel == "dev" else "-") + version
    if not isinstance(tag, str) or not tag.endswith(suffix):
        raise ValueError("タグと manifest の版が一致しません")
    dataset = tag[:-len(suffix)]
    if version_for_tag(dataset, tag, channel) != version:
        raise ValueError("タグと manifest の版が一致しません")
    return dataset


def check_branch(channel):
    if channel not in ("stable", "dev"):
        raise ValueError("配布先は stable または dev を指定してください")
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
    elif not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", target):
        raise ValueError("タグの形式が不正です")
    result = subprocess.run(
        ["gh", "api", "--paginate", "--slurp", f"repos/{repo}/releases?per_page=100"],
        check=True, capture_output=True, text=True, encoding="utf-8")
    entries = [entry for page in json.loads(result.stdout) for entry in page]
    matches = [entry for entry in entries
               if (entry.get("html_url") == target if is_url else entry["tag_name"] == target)]
    if len(matches) != 1:
        raise ValueError("指定した Release を一意に取得できません")
    release = matches[0]
    if release["prerelease"] != (channel == "dev"):
        raise ValueError("Release の正式版・開発版の区分が一致しません")
    with tempfile.TemporaryDirectory() as temporary:
        subprocess.run(["gh", "release", "download", release["tag_name"], "--repo", repo,
                        "--pattern", "manifest.json", "--dir", temporary], check=True)
        manifest = json.loads((Path(temporary) / "manifest.json").read_bytes())
    if not isinstance(manifest, dict):
        raise ValueError("manifest がオブジェクトではありません")
    if manifest.get("schemaVersion") == 6 and (
            manifest.get("schemaFormat") != "yamamuki-manifest"
            or manifest.get("dataSchemaFormat") != "yamamuki-points"):
        raise ValueError("未対応のデータ形式です")
    dataset = dataset_for_tag(release["tag_name"], channel, manifest.get("version"))
    return dataset, release["tag_name"]


def publish(dataset, tag, channel, sha256, reason):
    check_branch(channel)
    version_for_tag(dataset, tag, channel)
    subprocess.run([sys.executable, "-m", "release_tools.point_release", "--dataset", dataset, "--channel", channel,
                    "--tag", tag, "--sha256", sha256, "--manual", "--reason", reason],
                   cwd=ROOT, check=True)


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
