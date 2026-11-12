"""公開 API を模擬し、ビューワの検証と段階的な公開を確認する。"""

import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile

from release_tools import viewer_release as viewer
from release_tools import publish_reviewed as entry


TAG = "viewer-20261112T012345Z-123456-1"
RELEASE = dict(id=12, tag_name=TAG, draft=True, prerelease=False,
               target_commitish="a" * 40, body=viewer.site.TEMPORARY_DRAFT_MARKER,
               html_url=f"https://github.com/owner/repo/releases/tag/{TAG}")


def assets(directory, *, missing_executable=False):
    lines = []
    for platform, executable in (("windows-x64", "yamamuki-viewer.exe"),
                                 ("macos-arm64", "YamamukiViewer.app/Contents/MacOS/yamamuki-viewer")):
        name = f"{TAG}-{platform}.zip"
        with zipfile.ZipFile(directory / name, "w") as archive:
            archive.writestr("wrong" if missing_executable else executable, b"executable")
        lines.append(f"{hashlib.sha256((directory / name).read_bytes()).hexdigest()}  {name}\n")
    (directory / "SHA256SUMS.txt").write_text("".join(lines), encoding="utf-8")


class ViewerReleaseTests(unittest.TestCase):
    def test_existing_numbered_draft_still_accepts_preview(self):
        release = {**RELEASE, "tag_name": "viewer-v0.1.0"}
        with (patch.object(viewer.site, "gh", return_value="[]"),
              patch.object(viewer, "validate", side_effect=RuntimeError("download reached"))):
            with self.assertRaisesRegex(RuntimeError, "download reached"):
                viewer.publish_candidate(release, preview=True)

    def test_preview_then_formalize_preserves_release_and_assets_without_pages(self):
        release = dict(RELEASE)
        patches = []
        def gh(*args):
            if args[:2] == ("release", "download"):
                assets(Path(args[-1]))
                self.assertNotIn("manifest.json", args)
                return ""
            if args[:3] == ("api", "--method", "PATCH"):
                fields = json.loads(Path(args[-1]).read_text("utf-8"))
                patches.append(fields)
                release.update({key: value for key, value in fields.items() if key != "make_latest"})
                return json.dumps(release)
            return "[]"
        with (patch.dict(os.environ, GH_REPO="owner/repo"),
              patch.object(viewer.site, "gh", side_effect=gh),
              patch.object(viewer.site, "append_summary"),
              patch.object(viewer.site, "update_latest") as pages):
            viewer.publish_candidate(release, preview=True)
            self.assertTrue(release["prerelease"])
            self.assertNotIn(viewer.site.TEMPORARY_DRAFT_MARKER, release["body"])
            viewer.publish_candidate(release)
            viewer.publish_candidate(release)
        self.assertEqual(2, len(patches))
        self.assertFalse(release["draft"])
        self.assertFalse(release["prerelease"])
        self.assertEqual((12, TAG), (release["id"], release["tag_name"]))
        pages.assert_not_called()

    def test_invalid_zip_hash_or_checksum_file_stops(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            assets(directory)
            with self.assertRaises(ValueError):
                viewer.validate(directory, TAG, "0" * 64)
            (directory / f"{TAG}-windows-x64.zip").write_bytes(b"changed")
            with self.assertRaises(ValueError):
                viewer.validate(directory, TAG)
            assets(directory, missing_executable=True)
            with self.assertRaises(ValueError):
                viewer.validate(directory, TAG)
            assets(directory)
            sums = directory / "SHA256SUMS.txt"
            sums.write_text(sums.read_text("utf-8") * 2, encoding="utf-8")
            with self.assertRaises(ValueError):
                viewer.validate(directory, TAG)

    def test_wrong_states_and_branch_stop_before_download(self):
        for release, preview in ((RELEASE, False), ({**RELEASE, "draft": False}, True),
                                 ({**RELEASE, "tag_name": TAG + "-dev.1"}, True)):
            with patch.object(viewer.site, "gh") as gh, self.assertRaises(ValueError):
                viewer.publish_candidate(release, preview=preview)
            gh.assert_not_called()
        with (patch.dict(os.environ, GITHUB_ACTIONS="true", GITHUB_REF="refs/heads/feature"),
              patch.object(viewer.site, "gh") as gh, self.assertRaises(ValueError)):
            viewer.publish_candidate(RELEASE, preview=True)
        gh.assert_not_called()

    def test_changed_tag_commit_stops_before_patch(self):
        def gh(*args):
            if args[0] == "release":
                assets(Path(args[-1]))
                return ""
            return json.dumps([dict(ref=f"refs/tags/{TAG}", object=dict(sha="b" * 40))])
        with (patch.dict(os.environ, GH_REPO="owner/repo"),
              patch.object(viewer.site, "gh", side_effect=gh) as calls,
              self.assertRaises(ValueError)):
            viewer.publish_candidate(RELEASE, preview=True)
        self.assertFalse(any("PATCH" in call.args for call in calls.call_args_list))

    def test_common_actions_dispatch_viewer_without_point_manifest(self):
        for command in ("preview", "formalize"):
            with (patch("sys.argv", ["publish_reviewed", command, "--tag", TAG]),
                  patch.object(entry, "find_release", return_value=RELEASE),
                  patch.object(viewer, "publish_candidate") as publish):
                entry.main()
            publish.assert_called_once_with(RELEASE, "", preview=command == "preview")
