"""地表Draftの配布識別子を確定し、添付差し替えの途中からも再開する。"""

from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

from release_tools import release_data as site
from release_tools.publish_reviewed import ROOT

PATTERN = r"<!-- yamamuki-data:surface-identity:v1 (.+?) -->"
IDENTIFIER = r"[0-9]{8}T[0-9]{6}Z-[0-9]+-[0-9]+"


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def build_tool():
    subprocess.run(["cargo", "build", "--locked", "--release", "--manifest-path",
                    str(ROOT / "surface/Cargo.toml"), "--target-dir", str(ROOT / "surface/target"),
                    "--bin", "surface-tool"], check=True)
    return ROOT / "surface/target/release" / ("surface-tool.exe" if os.name == "nt" else "surface-tool")


def run_tool(tool, *args):
    result = subprocess.run([str(tool), *map(str, args)], capture_output=True,
                            text=True, encoding="utf-8")
    if result.returncode:
        raise RuntimeError(result.stderr.strip() or "RustのSQLite処理に失敗しました")
    return result.stdout


def write_json(path, document):
    path.write_text(json.dumps(document, ensure_ascii=False, sort_keys=True, indent=2) + "\n", encoding="utf-8")


def patch_release(release, fields, repo, directory):
    path = directory / "release-update.json"
    write_json(path, fields)
    result = json.loads(site.gh("api", "--method", "PATCH", f"repos/{repo}/releases/{release['id']}",
                                "--input", str(path)))
    if (result["id"] != release["id"] or not result["draft"] or result["prerelease"]
            or result["tag_name"] != fields["tag_name"]
            or result["target_commitish"] != fields["target_commitish"]):
        raise ValueError("同じ非公開Draftに識別子を記録できませんでした")
    return result


def finalize(release, manifest, dataset, package, settings, config, *, kind="surface"):
    records = re.findall(PATTERN.replace("surface-identity", f"{kind}-identity"), release.get("body") or "")
    if len(records) > 1:
        raise ValueError("配布識別子の確定記録が重複しています")
    plan = json.loads(records[0]) if records else None
    if records and not isinstance(plan, dict):
        raise ValueError("配布識別子の確定記録が不正です")
    # 既存の確定済み配布物の再検査は、識別子と添付を維持する。
    if (plan is None and re.fullmatch(IDENTIFIER, manifest.get("version", ""))
            and release["tag_name"] == config["releaseId"] + "-" + manifest["version"]):
        return release, manifest
    repo = os.environ["GH_REPO"]
    commit = release["target_commitish"]
    name = config["fileName"]
    manifest_name = Path(name).stem + ".manifest.json"
    names = (name, manifest_name, "LICENSE.txt")
    if plan is None:
        run_id, attempt = os.environ.get("GITHUB_RUN_ID", ""), os.environ.get("GITHUB_RUN_ATTEMPT", "")
        if not re.fullmatch(r"[0-9]+", run_id) or not re.fullmatch(r"[0-9]+", attempt):
            raise ValueError("識別子の確定にはActionsの実行IDと試行番号が必要です")
        identifier = f"{datetime.now(timezone.utc):%Y%m%dT%H%M%SZ}-{run_id}-{attempt}"
    else:
        identifier = plan.get("identifier", "")
        if (not re.fullmatch(IDENTIFIER, identifier) or plan.get("dataset") != dataset
                or plan.get("package") != package or plan.get("targetCommit") != commit
                or plan.get("tag") != release["tag_name"]):
            raise ValueError("識別子の確定記録と指定Draftが一致しません")
        for key in ("sourceFiles", "files"):
            if (not isinstance(plan.get(key), dict) or set(plan[key]) != set(names)
                    or any(not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value)
                           for value in plan[key].values())):
                raise ValueError("添付差し替えのハッシュ記録が不正です")
    tag = config["releaseId"] + "-" + identifier
    refs = json.loads(site.gh("api", f"repos/{repo}/git/matching-refs/tags/{tag}"))
    if any(ref["ref"] == "refs/tags/" + tag for ref in refs):
        raise ValueError("確定先のGitタグがすでに存在します")
    if any(entry["id"] != release["id"] and entry["tag_name"] == tag for entry in site.releases()):
        raise ValueError("確定先のReleaseタグがすでに使われています")
    with tempfile.TemporaryDirectory() as temporary:
        directory = Path(temporary)
        source, output = directory / "source", directory / "output"
        source.mkdir()
        output.mkdir()
        site.gh("release", "download", release["tag_name"], "--repo", repo,
                *(item for filename in names for item in ("--pattern", filename)), "--dir", str(source))
        current_hashes = {filename: digest(source / filename) for filename in names}
        current_manifest = json.loads((source / manifest_name).read_bytes())
        if plan:
            for filename in names:
                if current_hashes[filename] not in (plan["sourceFiles"][filename], plan["files"][filename]):
                    raise ValueError(f"記録にない添付の差し替えを検出しました: {filename}")
        elif (current_hashes[name] != manifest["sha256"] or current_manifest != manifest):
            raise ValueError("元のSQLiteとmanifestがダウンロード中に変化しました")
        tool = build_tool()
        # SQLiteを実際に読み、画像とコンテナを検査する。書き換えはRustで行う。
        run_tool(tool, "validate", source / name)
        timestamp = current_manifest["dataTimestamp"]
        source_manifest = json.loads(run_tool(tool, "manifest", source / name,
                                            f"https://github.com/{repo}/releases/download", timestamp, config["releaseId"]))
        if (len(source_manifest["datasets"]) != 1 or source_manifest["datasets"][0]["id"] != settings["datasetId"]
                or source_manifest["datasets"][0]["kind"] != kind):
            raise ValueError("元SQLiteのデータセットが公開設定と異なります")
        if source_manifest["version"] == identifier:
            shutil.copyfile(source / name, output / name)
        else:
            run_tool(tool, "set-data-version", source / name, output / name, identifier)
        result_manifest = json.loads(run_tool(tool, "manifest", output / name,
                                            f"https://github.com/{repo}/releases/download", timestamp, config["releaseId"]))
        result_manifest["licenseUrl"] = f"https://github.com/{repo}/releases/download/{tag}/LICENSE.txt"
        if (result_manifest["version"] != identifier or result_manifest["datasets"][0]["dataVersion"] != identifier):
            raise ValueError("SQLiteとmanifestの配布識別子が一致しません")
        write_json(output / manifest_name, result_manifest)
        license_lines = (source / "LICENSE.txt").read_text(encoding="utf-8-sig").splitlines()
        title = result_manifest["datasets"][0]["title"]
        (output / "LICENSE.txt").write_text(f"{title} {identifier}\n" + "\n".join(license_lines[1:]) + "\n", encoding="utf-8")
        final_hashes = {filename: digest(output / filename) for filename in names}
        if plan:
            if final_hashes != plan["files"]:
                raise ValueError("再実行で確定済みの配布内容が変化しました")
        else:
            plan = dict(identifier=identifier, tag=tag, dataset=dataset, package=package,
                        targetCommit=commit, sourceFiles=current_hashes, files=final_hashes)
            body = (site.TEMPORARY_DRAFT_MARKER + f"\n<!-- yamamuki-data:{kind}-identity:v1 "
                    + json.dumps(plan, ensure_ascii=False, separators=(",", ":")) + " -->\n\n"
                    + "配布識別子を確定しました。添付の加工・検査を実行中です。")
            release = patch_release(release, dict(tag_name=tag, target_commitish=commit, draft=True,
                                    prerelease=False, name="[draft] " + tag, body=body), repo, directory)
        for filename in names:
            if current_hashes[filename] != final_hashes[filename]:
                site.gh("release", "upload", tag, str(output / filename), "--repo", repo, "--clobber")
        print(f"確定した配布識別子: {identifier}", flush=True)
        return release, result_manifest
