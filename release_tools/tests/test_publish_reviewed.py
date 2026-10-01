"""共通入口の対象選択と種別ごとの振り分けを、公開せずに確認する。"""

import json
import os
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

    def test_draft_url_and_tag_select_same_dataset(self):
        release = dict(tag_name="peaks-dev-test", prerelease=True,
                       html_url="https://github.com/owner/repo/releases/tag/untagged-123")
        for target in (release["tag_name"], release["html_url"]):
            with self.subTest(target=target), patch.object(entry.subprocess, "run", return_value=SimpleNamespace(stdout=json.dumps([[], [release]]))) as run:
                self.assertEqual(("peaks", "peaks-dev-test"), entry.resolve(target, "dev"))
                self.assertIn("--paginate", run.call_args.args[0])

    def test_unsupported_and_cross_channel_tags_stop(self):
        for tag, channel in (("terrain-v1", "stable"), ("peaks-latest", "stable"),
                             ("peaks-dev-latest", "dev"), ("peaks-dev-v1", "stable"),
                             ("peaks-v1", "dev"), ("peaks-v1\ninjected=x", "stable")):
            with self.subTest(tag=tag), patch.object(entry.subprocess, "run") as run:
                with self.assertRaises(ValueError):
                    entry.resolve(tag, channel)
                run.assert_not_called()

    def test_foreign_url_stops_before_network(self):
        with patch.object(entry.subprocess, "run") as run, self.assertRaises(ValueError):
            entry.resolve("https://github.com/other/repo/releases/tag/peaks-v1", "stable")
        run.assert_not_called()

    def test_missing_ambiguous_or_wrong_prerelease_stops(self):
        release = dict(tag_name="peaks-v1", prerelease=True)
        for entries in ([], [release], [release, release]):
            with patch.object(entry.subprocess, "run", return_value=SimpleNamespace(stdout=json.dumps([entries]))):
                with self.assertRaises(ValueError):
                    entry.resolve("peaks-v1", "stable")

    def test_future_dataset_can_be_registered_without_another_workflow(self):
        with patch.dict(entry.PUBLISHERS, terrain=("terrain", "scripts.release_data")):
            self.assertEqual("terrain", entry.dataset_for_tag("terrain-dev-v1", "dev"))
            with patch.object(entry.subprocess, "run") as run:
                entry.publish("terrain", "terrain-dev-v1", "dev", "", "確認済み")
                self.assertEqual(entry.ROOT / "terrain", run.call_args.kwargs["cwd"])
                self.assertIn("terrain-dev-v1", run.call_args.args[0])

    def test_publish_passes_arguments_without_shell_and_propagates_failure(self):
        reason = "確認済み; $(command)"
        with patch.object(entry.subprocess, "run") as run:
            entry.publish("peaks", "peaks-dev-v1", "dev", "", reason)
            self.assertEqual(entry.ROOT / "peaks", run.call_args.kwargs["cwd"])
            self.assertNotIn("shell", run.call_args.kwargs)
            self.assertEqual(reason, run.call_args.args[0][-1])
            self.assertIn("--manual", run.call_args.args[0])
        with patch.object(entry.subprocess, "run", side_effect=subprocess.CalledProcessError(1, "publisher")):
            with self.assertRaises(subprocess.CalledProcessError):
                entry.publish("peaks", "peaks-v1", "stable", "", reason)

    def test_branch_dataset_and_reason_mismatch_stop_before_publish(self):
        for dataset, reason in (("terrain", "確認済み"), ("peaks", "")):
            with patch.object(entry.subprocess, "run") as run, self.assertRaises(ValueError):
                entry.publish(dataset, "peaks-v1", "stable", "", reason)
            run.assert_not_called()
        with patch.dict(os.environ, GITHUB_ACTIONS="true", GITHUB_REF="refs/heads/dev"), patch.object(entry.subprocess, "run") as run:
            with self.assertRaises(ValueError):
                entry.resolve("peaks-v1", "stable")
            with self.assertRaises(ValueError):
                entry.publish("peaks", "peaks-v1", "stable", "", "確認済み")
            run.assert_not_called()
