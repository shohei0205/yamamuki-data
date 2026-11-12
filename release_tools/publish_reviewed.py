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
    if os.environ.get("GITHUB_ACTIONS") == "true" and os.environ.get("GITHUB_REF") != "refs/heads/main":
        raise ValueError("公開は main ブランチから実行してください")


def find_release(target, entries=None):
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
    if entries is None:
        result = subprocess.run(
            ["gh", "api", "--paginate", "--slurp", f"repos/{repo}/releases?per_page=100"],
            check=True, capture_output=True, text=True, encoding="utf-8")
        entries = [entry for page in json.loads(result.stdout) for entry in page]
    matches = [entry for entry in entries
               if (entry.get("html_url") == target if is_url else entry["tag_name"] == target)]
    if len(matches) != 1:
        raise ValueError("指定した Release を一意に取得できません")
    return matches[0]


def resolve(target, channel):
    check_branch(channel)
    release = find_release(target)
    repo = os.environ["GH_REPO"]
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


def publish_candidate(target, sha256="", reason="", *, preview=False):
    check_branch("stable")
    release = find_release(target)
    if release["tag_name"].startswith("viewer-"):
        from release_tools.viewer_release import publish_candidate as publish_viewer
        publish_viewer(release, sha256, preview=preview)
        return
    repo = os.environ["GH_REPO"]
    with tempfile.TemporaryDirectory() as temporary:
        subprocess.run(["gh", "release", "download", release["tag_name"], "--repo", repo,
                        "--pattern", "manifest.json", "--dir", temporary], check=True)
        manifest = json.loads((Path(temporary) / "manifest.json").read_bytes())
    if not isinstance(manifest, dict) or manifest.get("schemaFormat") != "yamamuki-manifest":
        raise ValueError("未対応の manifest 形式です")
    if manifest.get("dataSchemaFormat") == "yamamuki-points":
        from release_tools.point_release import publish_candidate as publish_points
        publish_points(target, sha256, reason, preview=preview)
        return
    if manifest.get("dataSchemaFormat") != "yamamuki-sqlite" or manifest.get("schemaVersion") != 6:
        raise ValueError("未対応のデータ形式です")
    matches = re.findall(r"<!-- yamamuki-data:surface-draft:v1 (.+?) -->", release.get("body") or "")
    if len(matches) != 1:
        raise ValueError("地表の加工・検査記録がありません")
    receipt = json.loads(matches[0])
    if not isinstance(receipt, dict):
        raise ValueError("地表の加工・検査記録が不正です")
    dataset, package = receipt.get("dataset"), receipt.get("package")
    if (not isinstance(dataset, str) or not re.fullmatch(r"[a-z][a-z0-9_]*", dataset)
            or not isinstance(package, str) or not re.fullmatch(r"[a-z][a-z0-9_-]*", package)):
        raise ValueError("地表の配布設定の名前が不正です")
    checksum = sha256.strip() or receipt.get("sha256")
    commit = receipt.get("targetCommit")
    if (not isinstance(checksum, str) or not re.fullmatch(r"[0-9a-f]{64}", checksum)
            or not isinstance(commit, str) or not re.fullmatch(r"[0-9a-fA-F]{40}", commit)
            or checksum != manifest.get("sha256")):
        raise ValueError("地表のSHA-256またはコミットが一致しません")
    environment = os.environ.copy()
    environment["GITHUB_REPOSITORY"] = repo
    subprocess.run([
        "pwsh", "-NoProfile", "-File", str(ROOT / "surface/scripts/publish_release.ps1"),
        "-Dataset", dataset, "-Package", package,
        "-Operation", "Preview" if preview else "Formalize",
        "-Tag", release["tag_name"], "-Sha256", checksum, "-TargetCommit", commit,
        "-Reason", reason,
    ], cwd=ROOT, env=environment, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("resolve", "publish", "preview", "formalize"))
    parser.add_argument("--channel", choices=("stable", "dev"))
    parser.add_argument("--tag", required=True)
    parser.add_argument("--dataset")
    parser.add_argument("--sha256", default="")
    parser.add_argument("--reason", default="")
    args = parser.parse_args()
    if args.command in ("preview", "formalize"):
        if args.channel is not None:
            parser.error("正式公開の配布先は stable に固定されています")
        publish_candidate(args.tag, args.sha256, args.reason, preview=args.command == "preview")
        return
    if args.channel is None:
        parser.error("resolve と publish には --channel が必要です")
    if args.command == "resolve":
        dataset, tag = resolve(args.tag, args.channel)
        print(f"公開対象: {dataset} / {args.channel} / {tag}")
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
            stream.write(f"dataset={dataset}\ntag={tag}\n")
    else:
        publish(args.dataset, args.tag, args.channel, args.sha256, args.reason)


if __name__ == "__main__":
    main()
