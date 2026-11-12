"""Release のタグと manifest から、登録表なしで対象を判定する。"""

import json
import os
from pathlib import Path
import subprocess
import unittest
from types import SimpleNamespace
from unittest.mock import patch

from release_tools import publish_reviewed as entry


class PublishReviewedTests(unittest.TestCase):
    def setUp(self):
        environment = patch.dict(os.environ, GH_REPO="owner/repo", GITHUB_ACTIONS="false")
        environment.start()
        self.addCleanup(environment.stop)

    def resolve_with_manifest(self, target, release, manifest, channel):
        def run(arguments, **kwargs):
            if arguments[:2] == ["gh", "api"]:
                return SimpleNamespace(stdout=json.dumps([[], [release]]))
            self.assertEqual(["gh", "release", "download"], arguments[:3])
            directory = Path(arguments[-1])
            (directory / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
        with patch.object(entry.subprocess, "run", side_effect=run):
            return entry.resolve(target, channel)

    def test_url_and_tag_use_manifest_version_to_resolve_hyphenated_ids(self):
        for dataset in ("osm-peaks", "gsi-peaks", "new-custom-landmarks"):
            for channel in ("stable", "dev"):
                version = "20261018T105830Z-39121313669-1"
                tag = dataset + ("-dev-" if channel == "dev" else "-") + version
                release = dict(tag_name=tag, prerelease=channel == "dev",
                               html_url="https://github.com/owner/repo/releases/tag/untagged-123")
                manifest = dict(schemaVersion=6, schemaFormat="yamamuki-manifest",
                                dataSchemaFormat="yamamuki-points", version=version)
                for target in (tag, release["html_url"]):
                    with self.subTest(dataset=dataset, channel=channel, target=target):
                        self.assertEqual((dataset, tag), self.resolve_with_manifest(target, release, manifest, channel))

    def test_missing_ambiguous_or_wrong_prerelease_stops(self):
        release = dict(tag_name="osm-peaks-v1", prerelease=True)
        for entries in ([], [release], [release, release]):
            with patch.object(entry.subprocess, "run", return_value=SimpleNamespace(stdout=json.dumps([entries]))):
                with self.assertRaises(ValueError):
                    entry.resolve("osm-peaks-dev-v1", "dev")

    def test_foreign_url_and_injected_tag_stop_before_network(self):
        for target in ("https://github.com/other/repo/releases/tag/osm-peaks-v1", "osm-peaks-v1\ninjected=x"):
            with patch.object(entry.subprocess, "run") as run, self.assertRaises(ValueError):
                entry.resolve(target, "stable")
            run.assert_not_called()

    def test_invalid_manifest_or_tag_stops(self):
        for tag, version, channel in (("osm-peaks-latest", "latest", "stable"),
                ("osm-peaks-dev-latest", "latest", "dev"), ("osm-peaks-dev-v1", "v1", "stable"),
                ("osm-peaks-v1", "v1", "dev"), ("osm-peaks-v1", "other", "stable"),
                ("../peaks-v1", "v1", "stable"), ("osm-peaks-v1", None, "stable")):
            with self.subTest(tag=tag, version=version, channel=channel), self.assertRaises(ValueError):
                entry.dataset_for_tag(tag, channel, version)
        release = dict(tag_name="terrain-v1", prerelease=False)
        for manifest in ([], dict(version="v1", schemaVersion=6, schemaFormat="yamamuki-manifest", dataSchemaFormat="terrain")):
            with self.assertRaises(ValueError):
                self.resolve_with_manifest("terrain-v1", release, manifest, "stable")

    def test_new_dataset_publishes_without_registration_and_without_shell(self):
        reason = "確認済み; $(command)"
        with patch.object(entry.subprocess, "run") as run:
            entry.publish("landmarks", "landmarks-dev-v1", "dev", "", reason)
            self.assertEqual(entry.ROOT, run.call_args.kwargs["cwd"])
            self.assertIn("release_tools.point_release", run.call_args.args[0])
            self.assertNotIn("shell", run.call_args.kwargs)
            self.assertEqual(reason, run.call_args.args[0][-1])
        with patch.object(entry.subprocess, "run", side_effect=subprocess.CalledProcessError(1, "publisher")):
            with self.assertRaises(subprocess.CalledProcessError):
                entry.publish("landmarks", "landmarks-v1", "stable", "", reason)

    def test_empty_reason_is_passed_to_publisher(self):
        for reason in ("", "   "):
            with patch.object(entry.subprocess, "run") as run:
                entry.publish("osm-peaks", "osm-peaks-dev-v1", "dev", "", reason)
                self.assertEqual(reason, run.call_args.args[0][-1])

    def test_main_allows_both_channels(self):
        with patch.dict(os.environ, GITHUB_ACTIONS="true", GITHUB_REF="refs/heads/main"):
            for channel in ("stable", "dev"):
                with self.subTest(channel=channel):
                    entry.check_branch(channel)

    def test_branch_and_dataset_mismatch_stop_before_publish(self):
        with patch.object(entry.subprocess, "run") as run, self.assertRaises(ValueError):
            entry.publish("terrain", "osm-peaks-v1", "stable", "", "確認済み")
        run.assert_not_called()
        with patch.dict(os.environ, GITHUB_ACTIONS="true", GITHUB_REF="refs/heads/dev"), patch.object(entry.subprocess, "run") as run:
            with self.assertRaises(ValueError):
                entry.resolve("osm-peaks-dev-v1", "dev")
            with self.assertRaises(ValueError):
                entry.publish("osm-peaks", "osm-peaks-dev-v1", "dev", "", "確認済み")
            run.assert_not_called()

    def candidate_with_manifest(self, manifest, receipt, *, preview=True, checksum=""):
        release = dict(tag_name="esa-worldcover-japan-0.1.0",
                       body="<!-- yamamuki-data:surface-draft:v1 " + json.dumps(receipt) + " -->")
        commands = []

        def run(arguments, **kwargs):
            commands.append((arguments, kwargs))
            if arguments[:3] == ["gh", "release", "download"]:
                directory = Path(arguments[-1])
                (directory / arguments[arguments.index("--pattern") + 1]).write_text(json.dumps(manifest), encoding="utf-8")

        with patch.object(entry, "find_release", return_value=release), patch.object(entry.subprocess, "run", side_effect=run):
            entry.publish_candidate(release["tag_name"], checksum, "確認済み; $(command)", preview=preview)
        return commands

    def test_surface_candidates_use_the_same_preview_and_formalize_entry(self):
        manifest = dict(schemaFormat="yamamuki-manifest", schemaVersion=6,
                        dataSchemaFormat="yamamuki-sqlite", sha256="a" * 64)
        receipt = dict(dataset="esa_worldcover", package="all", sha256="a" * 64,
                       targetCommit="1" * 40)
        for preview in (True, False):
            commands = self.candidate_with_manifest(manifest, receipt, preview=preview)
            arguments, options = commands[-1]
            self.assertEqual(arguments[:2], ["pwsh", "-NoProfile"])
            self.assertIn("Preview" if preview else "Formalize", arguments)
            self.assertEqual(arguments[-1], "確認済み; $(command)")
            self.assertEqual(options["env"]["GITHUB_REPOSITORY"], "owner/repo")
            self.assertNotIn("shell", options)

    def test_surface_named_manifest_is_used_for_preview_and_formalize(self):
        manifest = dict(schemaFormat="yamamuki-manifest", schemaVersion=6,
                        dataSchemaFormat="yamamuki-sqlite", sha256="a" * 64)
        receipt = dict(dataset="esa_worldcover", package="all", sha256="a" * 64,
                       targetCommit="1" * 40, manifestName="esa-worldcover-z12.manifest.json")
        for preview in (True, False):
            commands = self.candidate_with_manifest(manifest, receipt, preview=preview)
            self.assertEqual(commands[0][0][commands[0][0].index("--pattern") + 1], receipt["manifestName"])
        for name in ("../manifest.json", "/manifest.json", "a/b.json", "a\\b.json", "a b.json", ""):
            with self.subTest(name=name), self.assertRaises(ValueError):
                self.candidate_with_manifest(manifest, {**receipt, "manifestName": name})

    def test_surface_candidate_rejects_invalid_checksum_and_config_paths(self):
        manifest = dict(schemaFormat="yamamuki-manifest", schemaVersion=6,
                        dataSchemaFormat="yamamuki-sqlite", sha256="a" * 64)
        receipt = dict(dataset="esa_worldcover", package="all", sha256="a" * 64,
                       targetCommit="1" * 40)
        for changes, checksum in [({}, "b" * 64), ({"dataset": "../other"}, ""),
                                  ({"package": "all; command"}, ""), ({"targetCommit": "main"}, "")]:
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                self.candidate_with_manifest(manifest, {**receipt, **changes}, checksum=checksum)

    def test_point_candidate_keeps_the_existing_publication_flow(self):
        manifest = dict(schemaFormat="yamamuki-manifest", schemaVersion=6,
                        dataSchemaFormat="yamamuki-points")
        with patch("release_tools.point_release.publish_candidate") as publish:
            self.candidate_with_manifest(manifest, {}, preview=False)
            self.assertEqual(publish.call_args.kwargs, {"preview": False})
