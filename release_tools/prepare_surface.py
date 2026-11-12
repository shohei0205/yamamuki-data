"""下書きReleaseの指定だけから、地表の加工・検査に必要な情報を取得する。"""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

from release_tools.publish_reviewed import ROOT, check_branch, find_release
from release_tools.surface_identity import finalize


def resolve_config(tag):
    matches = []
    for path in sorted((ROOT / "surface").glob("*/release.json")):
        settings = json.loads(path.read_text(encoding="utf-8-sig"))
        for package, config in settings["packages"].items():
            prefix = config["releaseId"] + "-"
            if tag.startswith(prefix) and re.fullmatch(settings["versionPattern"], tag[len(prefix):]):
                matches.append((path.parent.name, package, settings, config))
    if len(matches) != 1:
        raise ValueError("下書きのタグに対応する地表の公開設定を一意に特定できません")
    return matches[0]


def prepare(target):
    check_branch("stable")
    release = find_release(target)
    if not release.get("draft") or release.get("prerelease"):
        raise ValueError("未公開のDraft Releaseを指定してください")
    tag = release["tag_name"]
    dataset, package, settings, config = resolve_config(tag)
    commit = release.get("target_commitish", "")
    if not isinstance(commit, str) or not re.fullmatch(r"[0-9a-fA-F]{40}", commit):
        raise ValueError("下書きの対象コミットが40桁のSHAで固定されていません")
    filename = config["fileName"]
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*\.sqlite3", filename):
        raise ValueError("公開設定のSQLiteファイル名が不正です")
    manifest_name = Path(filename).stem + ".manifest.json"
    with tempfile.TemporaryDirectory() as temporary:
        subprocess.run(["gh", "release", "download", tag, "--repo", os.environ["GH_REPO"],
                        "--pattern", manifest_name, "--dir", temporary], check=True)
        manifest = json.loads((Path(temporary) / manifest_name).read_bytes())
    checksum = manifest.get("sha256", "")
    datasets = manifest.get("datasets")
    if (manifest.get("schemaFormat") != "yamamuki-manifest" or manifest.get("schemaVersion") != 6
            or manifest.get("dataSchemaFormat") != "yamamuki-sqlite"
            or manifest.get("dataSchemaVersion") != 1 or manifest.get("fileName") != filename
            or not isinstance(checksum, str) or not re.fullmatch(r"[0-9a-f]{64}", checksum)
            or not isinstance(datasets, list) or len(datasets) != 1
            or datasets[0].get("id") != settings["datasetId"]
            or datasets[0].get("kind") != "surface"):
        raise ValueError("添付manifestと地表の公開設定が一致しません")
    release, manifest = finalize(release, manifest, dataset, package, settings, config)
    tag, checksum = release["tag_name"], manifest["sha256"]
    print(f"加工対象: {dataset}/{package} / {tag}", flush=True)
    print(f"対象コミット: {commit}\nSQLiteのSHA-256: {checksum}", flush=True)
    environment = dict(os.environ, GITHUB_REPOSITORY=os.environ["GH_REPO"])
    subprocess.run(["pwsh", "-NoProfile", "-File", str(ROOT / "surface/scripts/publish_release.ps1"),
                    "-Dataset", dataset, "-Package", package, "-Operation", "Prepare",
                    "-Tag", tag, "-Sha256", checksum, "-TargetCommit", commit],
                   check=True, env=environment)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, help="下書きReleaseのURLまたはタグ")
    prepare(parser.parse_args().target)


if __name__ == "__main__":
    main()
