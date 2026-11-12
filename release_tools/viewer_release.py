"""ビューワの配布 ZIP を検証し、同じ Release を段階的に公開する。"""

import hashlib
import json
from pathlib import Path
import re
import tempfile
import zipfile

from release_tools import release_data as site
from release_tools.publish_reviewed import check_branch


def validate(directory, tag, expected_sha256=""):
    names = [f"{tag}-{platform}.zip" for platform in ("windows-x64", "macos-arm64")]
    sums = (directory / "SHA256SUMS.txt").read_bytes()
    if expected_sha256 and hashlib.sha256(sums).hexdigest() != expected_sha256:
        raise ValueError("SHA256SUMS.txt の SHA-256 が指定値と一致しません")
    entries = {}
    for line in sums.decode("utf-8").splitlines():
        match = re.fullmatch(r"([a-f0-9]{64})  (.+)", line)
        if not match or match[2] not in names or match[2] in entries:
            raise ValueError("SHA256SUMS.txt の形式・添付ファイル名が不正です")
        entries[match[2]] = match[1]
    if set(entries) != set(names):
        raise ValueError("2 環境の ZIP の SHA-256 が必要です")
    for name in names:
        raw = (directory / name).read_bytes()
        if hashlib.sha256(raw).hexdigest() != entries[name]:
            raise ValueError(f"ZIP の SHA-256 が一致しません: {name}")
        with zipfile.ZipFile(directory / name) as archive:
            executable = ("yamamuki-viewer.exe" if name.endswith("windows-x64.zip") else
                          "YamamukiViewer.app/Contents/MacOS/yamamuki-viewer")
            if archive.testzip() is not None or executable not in archive.namelist() or not archive.getinfo(executable).file_size:
                raise ValueError(f"ZIP に正常な実行ファイルがありません: {name}")
    return hashlib.sha256(sums).hexdigest()


def publish_candidate(release, sha256="", *, preview=False):
    check_branch("stable")
    tag = release["tag_name"]
    if not re.fullmatch(r"viewer-(?:\d{8}T\d{6}Z-\d+-\d+|v\d+\.\d+\.\d+)", tag):
        raise ValueError("ビューワのタグは viewer-<生成日時>-<実行 ID>-<試行番号> を指定してください")
    if release["draft"] and not preview:
        raise ValueError("下書きを先に試用向けに公開してください")
    if preview and not (release["draft"] or release["prerelease"]):
        raise ValueError("正式公開済みの Release は試用版に戻せません")
    with tempfile.TemporaryDirectory() as temporary:
        directory = Path(temporary)
        names = [f"{tag}-{platform}.zip" for platform in ("windows-x64", "macos-arm64")]
        site.gh("release", "download", tag,
                *(part for name in [*names, "SHA256SUMS.txt"] for part in ("--pattern", name)),
                "--dir", str(directory))
        checksum = validate(directory, tag, sha256.strip())
        fields = {"draft": False, "prerelease": preview, "make_latest": "false"}
        if preview:
            fields["name"] = (release.get("name") or tag).removeprefix("draft-")
        if release["draft"]:
            commit = release["target_commitish"]
            if not re.fullmatch(r"[0-9a-f]{40}", commit):
                raise ValueError("下書きの作成元コミットが SHA で固定されていません")
            refs = json.loads(site.gh("api", site.endpoint(f"git/matching-refs/tags/{tag}")))
            if any(ref["ref"] == f"refs/tags/{tag}" and ref["object"]["sha"] != commit for ref in refs):
                raise ValueError("タグが下書きの作成元と異なるコミットを指しています")
            fields.update(target_commitish=commit,
                          body=(release.get("body") or "").replace(site.TEMPORARY_DRAFT_MARKER, ""))
        published = release
        if release["draft"] or release["prerelease"] != preview:
            request = directory / "publish.json"
            request.write_text(json.dumps(fields), encoding="utf-8")
            published = json.loads(site.gh("api", "--method", "PATCH",
                                          site.endpoint(f"releases/{release['id']}"), "--input", str(request)))
        if (published["id"] != release["id"] or published["tag_name"] != tag
                or published["draft"] or published["prerelease"] != preview):
            raise ValueError("同じ Release が指定した状態で公開されたことを確認できません")
        kind = "試用公開" if preview else "正式公開"
        site.append_summary(f"\n## ビューワの{kind}\n\n[Release を開く]({published['html_url']})\n\n"
                            f"- タグ: `{tag}`\n- SHA256SUMS.txt の SHA-256: `{checksum}`\n")
