"""識別子確定と添付差し替えの途中失敗・再実行を、GitHubに接続せず検証する。"""

import copy
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import unittest
from unittest.mock import patch

from release_tools import surface_identity as target


def encoded(document):
    return (json.dumps(document, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode()


class SurfaceIdentityTests(unittest.TestCase):
    def setUp(self):
        environment = patch.dict(os.environ, GH_REPO="owner/repo", GITHUB_RUN_ID="12345", GITHUB_RUN_ATTEMPT="1")
        environment.start()
        self.addCleanup(environment.stop)
        self.config = dict(fileName="esa-worldcover-z12.sqlite3", releaseId="esa-worldcover")
        self.settings = dict(datasetId="esa-worldcover-japan")
        self.release = dict(id=42, tag_name="esa-worldcover-20261110T032217Z", draft=True,
                            prerelease=False, target_commitish="a" * 40, body=target.site.TEMPORARY_DRAFT_MARKER)
        database = encoded(dict(version="20261110T032217Z", images="unchanged"))
        self.files = {self.config["fileName"]: database, "LICENSE.txt": b"Old title\n\nlicense text\n"}
        self.manifest = self.make_manifest(database)
        self.files["esa-worldcover-z12.manifest.json"] = encoded(self.manifest)
        self.uploads, self.patches = [], []
        self.fail_upload = 0

    def make_manifest(self, database):
        version = json.loads(database)["version"]
        name = self.config["fileName"]
        return dict(version=version, sha256=hashlib.sha256(database).hexdigest(), fileName=name,
                    sizeBytes=len(database), dataTimestamp="2026-11-10T03:22:17Z",
                    downloadUrl=f"https://github.com/owner/repo/releases/download/esa-worldcover-{version}/{name}",
                    datasets=[dict(id="esa-worldcover-japan", kind="surface", dataVersion=version, title="ESA")])

    def gh(self, *args):
        if args[:2] == ("api", "--method"):
            self.assertEqual("PATCH", args[2])
            fields = json.loads(Path(args[-1]).read_bytes())
            self.assertTrue(fields["draft"])
            self.assertFalse(fields["prerelease"])
            self.assertEqual("a" * 40, fields["target_commitish"])
            self.patches.append(fields)
            self.release.update(fields)
            return json.dumps(self.release)
        if args[0] == "api":
            self.assertIn("matching-refs/tags/", args[1])
            return "[]"
        if args[:2] == ("release", "download"):
            self.assertEqual(self.release["tag_name"], args[2])
            directory = Path(args[-1])
            for name, data in self.files.items():
                (directory / name).write_bytes(data)
            return ""
        if args[:2] == ("release", "upload"):
            self.assertEqual(self.release["tag_name"], args[2])
            self.assertIn("surface-identity:v1", self.release["body"])
            self.uploads.append(Path(args[3]).name)
            if len(self.uploads) == self.fail_upload:
                raise RuntimeError("転送中断")
            self.files[Path(args[3]).name] = Path(args[3]).read_bytes()
            return ""
        raise AssertionError(args)

    def tool(self, tool, *args):
        if args[0] == "validate":
            return "checked"
        if args[0] == "set-data-version":
            document = json.loads(Path(args[1]).read_bytes())
            document["version"] = args[3]
            Path(args[2]).write_bytes(encoded(document))
            return ""
        self.assertEqual("manifest", args[0])
        return json.dumps(self.make_manifest(Path(args[1]).read_bytes()))

    def run_finalize(self):
        current_manifest = json.loads(self.files["esa-worldcover-z12.manifest.json"])
        with patch.object(target.site, "gh", side_effect=self.gh), \
                patch.object(target.site, "releases", return_value=[self.release]), \
                patch.object(target, "build_tool", return_value="tool"), \
                patch.object(target, "run_tool", side_effect=self.tool), \
                patch.object(target, "datetime") as clock:
            clock.now.return_value = datetime(2026, 11, 13, 1, 2, 3, tzinfo=timezone.utc)
            return target.finalize(copy.deepcopy(self.release), current_manifest,
                                   "esa_worldcover", "all", self.settings, self.config)

    def test_one_identifier_for_release_database_manifest_and_urls(self):
        release, manifest = self.run_finalize()
        identifier = "20261113T010203Z-12345-1"
        self.assertEqual(identifier, manifest["version"])
        self.assertEqual(identifier, manifest["datasets"][0]["dataVersion"])
        self.assertEqual(identifier, json.loads(self.files[self.config["fileName"]])["version"])
        self.assertEqual("unchanged", json.loads(self.files[self.config["fileName"]])["images"])
        self.assertEqual("esa-worldcover-" + identifier, release["tag_name"])
        self.assertEqual("[draft] " + release["tag_name"], release["name"])
        self.assertIn(release["tag_name"], manifest["downloadUrl"])
        self.assertEqual("2026-11-10T03:22:17Z", manifest["dataTimestamp"])
        self.assertEqual(3, len(self.uploads))

    def test_partial_upload_reuses_recorded_identity_on_another_attempt(self):
        self.fail_upload = 2
        with self.assertRaisesRegex(RuntimeError, "転送中断"):
            self.run_finalize()
        identifier = self.release["tag_name"]
        self.assertEqual(1, len(self.patches))
        self.fail_upload = 0
        with patch.dict(os.environ, GITHUB_RUN_ID="99999", GITHUB_RUN_ATTEMPT="2"):
            release, manifest = self.run_finalize()
        self.assertEqual(identifier, release["tag_name"])
        self.assertTrue(manifest["version"].endswith("-12345-1"))
        self.assertEqual(1, len(self.patches))
        self.assertEqual(1, self.uploads.count(self.config["fileName"]))
        uploads = len(self.uploads)
        self.run_finalize()
        self.assertEqual(uploads, len(self.uploads))

    def test_unknown_attachment_change_is_rejected_during_resume(self):
        self.fail_upload = 1
        with self.assertRaises(RuntimeError):
            self.run_finalize()
        self.files["LICENSE.txt"] = b"changed outside the action"
        with self.assertRaisesRegex(ValueError, "記録にない添付"):
            self.run_finalize()

    def test_record_cannot_redirect_to_another_dataset(self):
        self.fail_upload = 1
        with self.assertRaises(RuntimeError):
            self.run_finalize()
        record = json.loads(re.findall(target.PATTERN, self.release["body"])[0])
        record["dataset"] = "different"
        self.release["body"] = "<!-- yamamuki-data:surface-identity:v1 " + json.dumps(record) + " -->"
        with self.assertRaisesRegex(ValueError, "一致しません"):
            self.run_finalize()

    def test_invalid_record_is_rejected_without_mutation(self):
        for record in ([], "invalid", 1):
            with self.subTest(record=record):
                self.release["body"] = "<!-- yamamuki-data:surface-identity:v1 " + json.dumps(record) + " -->"
                with self.assertRaisesRegex(ValueError, "記録が不正"):
                    self.run_finalize()
        self.assertEqual([], self.patches)
        self.assertEqual([], self.uploads)
