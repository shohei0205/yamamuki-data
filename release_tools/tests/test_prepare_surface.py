"""地表DraftのURLだけで入力を解決し、不一致時には加工しないことを確認する。"""

import copy
import json
import os
from pathlib import Path
import unittest
from unittest.mock import patch

from release_tools import prepare_surface as target


class PrepareSurfaceTests(unittest.TestCase):
    def setUp(self):
        environment = patch.dict(os.environ, GH_REPO="owner/repo", GITHUB_ACTIONS="true",
                                 GITHUB_REF="refs/heads/main")
        environment.start()
        self.addCleanup(environment.stop)
        self.release = dict(tag_name="esa-worldcover-20261112T000000Z-12345-1", draft=True,
                            prerelease=False, target_commitish="a" * 40)
        self.manifest = dict(schemaFormat="yamamuki-manifest", schemaVersion=6,
                             dataSchemaFormat="yamamuki-sqlite", dataSchemaVersion=1,
                             fileName="esa-worldcover-z12.sqlite3", sha256="b" * 64,
                             datasets=[dict(id="esa-worldcover-japan", kind="surface")])

    def invoke(self, release=None, manifest=None):
        release = self.release if release is None else release
        manifest = self.manifest if manifest is None else manifest
        def run(args, **kwargs):
            if args[0] == "gh":
                (Path(args[-1]) / "esa-worldcover-z12.manifest.json").write_text(
                    json.dumps(manifest), encoding="utf-8")
        with patch.object(target, "find_release", return_value=release) as resolve, \
                patch.object(target.subprocess, "run", side_effect=run) as calls:
            target.prepare("https://github.com/owner/repo/releases/tag/untagged-test")
        resolve.assert_called_once_with("https://github.com/owner/repo/releases/tag/untagged-test")
        return calls

    def test_url_resolves_all_processing_inputs(self):
        calls = self.invoke()
        self.assertEqual(2, calls.call_count)
        args = calls.call_args.args[0]
        for key, value in (("-Dataset", "esa_worldcover"), ("-Package", "all"),
                           ("-Tag", self.release["tag_name"]), ("-Sha256", "b" * 64),
                           ("-TargetCommit", "a" * 40), ("-Operation", "Prepare")):
            self.assertEqual(value, args[args.index(key) + 1])
        self.assertEqual("owner/repo", calls.call_args.kwargs["env"]["GITHUB_REPOSITORY"])

    def test_invalid_draft_stops_before_download(self):
        for change in (dict(draft=False), dict(prerelease=True), dict(target_commitish="main"),
                       dict(tag_name="unknown-0.1.0"), dict(tag_name="esa-worldcover-0.1.0")):
            with self.subTest(change=change), patch.object(target, "find_release", return_value={**self.release, **change}), \
                    patch.object(target.subprocess, "run") as calls:
                with self.assertRaises(ValueError):
                    target.prepare(self.release["tag_name"])
                calls.assert_not_called()

    def test_invalid_manifest_stops_before_processing(self):
        for change in (dict(sha256="invalid"), dict(fileName="other.sqlite3"),
                       dict(schemaVersion=5), dict(dataSchemaVersion=2),
                       dict(datasets=[dict(id="different", kind="surface")])):
            with self.subTest(change=change):
                manifest = copy.deepcopy(self.manifest)
                manifest.update(change)
                with patch.object(target, "find_release", return_value=self.release), \
                        patch.object(target.subprocess, "run") as calls:
                    def download(args, **kwargs):
                        self.assertEqual("gh", args[0])
                        (Path(args[-1]) / "esa-worldcover-z12.manifest.json").write_text(
                            json.dumps(manifest), encoding="utf-8")
                    calls.side_effect = download
                    with self.assertRaises(ValueError):
                        target.prepare(self.release["tag_name"])
                    self.assertEqual(1, calls.call_count)
