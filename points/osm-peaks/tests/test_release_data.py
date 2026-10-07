"""GitHub に書き込まず、自動公開・手動公開・保留の分岐を確かめる。"""

import os
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from scripts import release_data
from test_check_release import dataset


class ReleaseDataTests(unittest.TestCase):
    def setUp(self):
        self.current = dataset()
        self.current[0].update(sha256="a" * 64, sizeBytes=123, mountainCount=len(self.current[1]))
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.test_output = Path(temporary.name)
        self.environment = patch.dict(os.environ, {"GH_REPO": "owner/repo", "GITHUB_SHA": "commit", "GITHUB_ACTOR": "tester", "GITHUB_ACTIONS": "false", "GITHUB_STEP_SUMMARY": str(self.test_output / "summary.md"), "GITHUB_OUTPUT": str(self.test_output / "output")})
        self.environment.start()
        self.addCleanup(self.environment.stop)

    def run_publish(self, *, warnings=None, manual=False, expected=None, draft=True):
        with patch.object(release_data, "gh", return_value=json.dumps([[dict(tag_name='osm-peaks-test', draft=draft, prerelease=False)]])) as gh, \
                patch.object(release_data, "fetch", return_value=self.current), \
                patch.object(release_data, "previous_release", return_value=self.current), \
                patch.object(release_data, "assess", return_value=warnings or []), \
                patch.object(release_data, "update_latest") as update:
            release_data.publish("osm-peaks-test", expected or "a" * 64, manual=manual, reason="意図した減少を確認")
            self.assertEqual(update.called, manual or not warnings)
            return [call.args for call in gh.call_args_list]

    def test_normal_data_published(self):
        calls = self.run_publish()
        self.assertIn(("release", "edit", "osm-peaks-test", "--draft=false", "--latest=false"), calls)
        self.assertIn("[公開したリリースを開く](https://github.com/owner/repo/releases/tag/osm-peaks-test)",
                      (self.test_output / "summary.md").read_text(encoding="utf-8"))

    def test_generated_release_link_uses_url_returned_by_github(self):
        for warnings in ([], ["初回"]):
            with self.subTest(warnings=warnings), tempfile.TemporaryDirectory() as directory, \
                    patch.object(release_data, "validate", return_value=self.current), \
                    patch.object(release_data, "previous_release", return_value=self.current), \
                    patch.object(release_data, "assess", return_value=warnings), \
                    patch.object(release_data, "gh", return_value="https://github.com/owner/repo/releases/tag/untagged-123\n"):
                summary_path = self.test_output / "summary.md"
                summary_path.write_text("", encoding="utf-8")
                release_data.prepare(directory, "dev")
                summary = summary_path.read_text(encoding="utf-8")
                self.assertIn("[生成したリリースを開く](https://github.com/owner/repo/releases/tag/untagged-123)", summary)
                self.assertIn("osm-peaks-dev-test", summary)
                self.assertIn("下書きで保留" if warnings else "自動公開の段階", summary)

    def test_release_title_matches_tag(self):
        for channel in ("stable", "dev"):
            with self.subTest(channel=channel), tempfile.TemporaryDirectory() as directory, \
                    patch.object(release_data, "validate", return_value=self.current), \
                    patch.object(release_data, "previous_release", return_value=self.current), \
                    patch.object(release_data, "assess", return_value=[]), \
                    patch.object(release_data, "gh", return_value="https://example.com/release") as gh:
                release_data.prepare(directory, channel)
                arguments = gh.call_args.args
                self.assertEqual(arguments[arguments.index("--title") + 1], arguments[2])
                self.assertEqual(arguments[2], "osm-peaks-" + ("dev-" if channel == "dev" else "") + "test")

    def test_failed_creation_does_not_add_release_link(self):
        with tempfile.TemporaryDirectory() as directory, \
                patch.object(release_data, "validate", return_value=self.current), \
                patch.object(release_data, "previous_release", return_value=None), \
                patch.object(release_data, "gh", side_effect=RuntimeError("作成失敗")):
            with self.assertRaises(RuntimeError):
                release_data.prepare(directory)
            self.assertNotIn("生成したリリースを開く", (self.test_output / "summary.md").read_text(encoding="utf-8"))

    def test_warning_keeps_draft(self):
        calls = self.run_publish(warnings=["件数減少"])
        self.assertFalse(any("--draft=false" in call for call in calls))

    def test_manual_review_overrides_warning(self):
        calls = self.run_publish(warnings=["初回", "件数減少"], manual=True)
        self.assertIn(("release", "edit", "osm-peaks-test", "--draft=false", "--latest=false"), calls)
        self.assertFalse(any("upload" in call or "create" in call for call in calls))

    def test_wrong_checksum_and_published_release_stop(self):
        with self.assertRaises(ValueError):
            self.run_publish(manual=True, expected="b" * 64)
        with self.assertRaises(ValueError):
            self.run_publish(draft=False)

    def test_manual_reason_optional_and_recorded(self):
        for reason in ("", "   ", "件数を確認"):
            with self.subTest(reason=reason), \
                    patch.object(release_data, "find_release", return_value=dict(tag_name="osm-peaks-test", draft=True, prerelease=False)), \
                    patch.object(release_data, "gh"), \
                    patch.object(release_data, "fetch", return_value=self.current), \
                    patch.object(release_data, "previous_release", return_value=None), \
                    patch.object(release_data, "write_report") as report, \
                    patch.object(release_data, "update_latest") as update:
                release_data.publish("osm-peaks-test", "a" * 64, manual=True, reason=reason)
                update.assert_called_once()
                self.assertIn("確認者: tester", report.call_args.args[1])
                self.assertIn(reason.strip() or "理由の記入なし", report.call_args.args[1])

    def test_corrupt_asset_and_baseline_failure_never_publish(self):
        for method in ("fetch", "previous_release"):
            with self.subTest(method=method), \
                    patch.object(release_data, "gh", return_value='[[{"tag_name": "osm-peaks-test", "draft": true, "prerelease": false}]]') as gh, \
                    patch.object(release_data, "fetch", return_value=self.current), \
                    patch.object(release_data, "previous_release", return_value=self.current), \
                    patch.object(release_data, method, side_effect=ValueError("検証失敗")):
                with self.assertRaises(ValueError):
                    release_data.publish("osm-peaks-test", "a" * 64, manual=True, reason="確認済み")
                self.assertFalse(any("--draft=false" in call.args for call in gh.call_args_list))

    def test_prepare_first_release_or_failed_comparison_keeps_draft(self):
        for failure in (False, True):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory, \
                    patch.dict(os.environ, {"GITHUB_OUTPUT": str(Path(directory) / "output")}), \
                    patch.object(release_data, "validate", return_value=self.current), \
                    patch.object(release_data, "previous_release", return_value=None,
                                 side_effect=RuntimeError("通信失敗") if failure else None), \
                    patch.object(release_data, "gh", return_value="https://github.com/owner/repo/releases/tag/untagged-123\n") as gh:
                release_data.prepare(directory)
                self.assertIn("auto_publish=false", (Path(directory) / "output").read_text())
                self.assertIn("--draft", gh.call_args.args)
                self.assertNotIn("--draft=false", gh.call_args.args)


    def test_invalid_tag_never_calls_github(self):
        with patch.object(release_data, "gh") as gh:
            with self.assertRaises(ValueError):
                release_data.publish("--bad-tag", "a" * 64)
            gh.assert_not_called()

    def test_manual_can_repair_published_pointer(self):
        calls = self.run_publish(manual=True, draft=False)
        self.assertFalse(any("--draft=false" in call for call in calls))




    def test_reserved_pointer_tag_rejected(self):
        for tag in ("osm-peaks-latest", "terrain-test", "data-test"):
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                release_data.check_tag(tag)

    def test_published_repair_can_restore_missing_pointer_asset(self):
        with patch.object(release_data, "gh", return_value='[[{"tag_name": "osm-peaks-test", "draft": false, "prerelease": false}]]'), \
                patch.object(release_data, "fetch", return_value=self.current), \
                patch.object(release_data, "previous_release", side_effect=RuntimeError("manifest 欠落")), \
                patch.object(release_data, "update_latest") as update:
            release_data.publish("osm-peaks-test", "a" * 64, manual=True, reason="参照先更新の失敗を確認")
            update.assert_called_once()

    def test_failed_publication_never_updates_pointer(self):
        def gh(*args):
            if "--draft=false" in args:
                raise RuntimeError("公開失敗")
            return '[[{"tag_name": "osm-peaks-test", "draft": true, "prerelease": false}]]'
        with patch.object(release_data, "gh", side_effect=gh), \
                patch.object(release_data, "fetch", return_value=self.current), \
                patch.object(release_data, "previous_release", return_value=None), \
                patch.object(release_data, "update_latest") as update:
            with self.assertRaises(RuntimeError):
                release_data.publish("osm-peaks-test", "a" * 64, manual=True, reason="確認済み")
            update.assert_not_called()

    def test_dev_preparation_creates_prerelease_draft(self):
        with tempfile.TemporaryDirectory() as directory, \
                patch.object(release_data, "validate", return_value=self.current), \
                patch.object(release_data, "previous_release", return_value=None) as previous, \
                patch.object(release_data, "gh", return_value="https://github.com/owner/repo/releases/tag/untagged-123\n") as gh:
            release_data.prepare(directory, "dev")
            self.assertEqual("dev", previous.call_args.args[1])
            self.assertEqual("osm-peaks-dev-test", gh.call_args.args[2])
            self.assertIn("--prerelease=true", gh.call_args.args)
            self.assertIn("--draft", gh.call_args.args)

    def test_dev_publish_and_warning_use_only_dev_channel(self):
        for warnings, manual in (([], False), (["更新なし"], False), (["件数減少"], True)):
            with self.subTest(warnings=warnings, manual=manual), \
                    patch.object(release_data, "gh", return_value='[[{"tag_name": "osm-peaks-dev-test", "draft": true, "prerelease": true}]]') as gh, \
                    patch.object(release_data, "fetch", return_value=self.current) as fetch, \
                    patch.object(release_data, "previous_release", return_value=self.current) as previous, \
                    patch.object(release_data, "assess", return_value=warnings), \
                    patch.object(release_data, "update_latest") as update:
                release_data.publish("osm-peaks-dev-test", "a" * 64, channel="dev", manual=manual, reason="確認済み")
                self.assertEqual("dev", fetch.call_args.args[2])
                self.assertEqual("dev", previous.call_args.args[1])
                self.assertEqual(manual or not warnings, update.called)
                if update.called:
                    self.assertEqual("dev", update.call_args.args[2])
                self.assertFalse(any("osm-peaks-test" in call.args for call in gh.call_args_list))

    def test_cross_channel_tags_rejected_before_github(self):
        for tag, channel in (("osm-peaks-test", "dev"), ("osm-peaks-dev-test", "stable"),
                             ("osm-peaks-dev-latest", "dev"), ("osm-peaks-test", "unknown")):
            with self.subTest(tag=tag, channel=channel), patch.object(release_data, "gh") as gh:
                with self.assertRaises(ValueError):
                    release_data.publish(tag, "a" * 64, channel=channel)
                gh.assert_not_called()

    def test_prerelease_mismatch_stops_before_downloading(self):
        for channel, tag, prerelease in (("stable", "osm-peaks-test", True), ("dev", "osm-peaks-dev-test", False)):
            with self.subTest(channel=channel), \
                    patch.object(release_data, "gh", return_value=json.dumps([[dict(tag_name=tag, draft=True, prerelease=prerelease)]])), \
                    patch.object(release_data, "fetch") as fetch:
                with self.assertRaises(ValueError):
                    release_data.publish(tag, "a" * 64, channel=channel)
                fetch.assert_not_called()



    def test_actions_branch_channel_mismatch_stops_before_io(self):
        for branch, channel in (("main", "dev"), ("dev", "stable"), ("feature", "stable"),
                                ("feature", "dev"), ("test", "dev"), ("test", "stable")):
            with self.subTest(branch=branch, channel=channel), \
                    patch.dict(os.environ, {"GITHUB_ACTIONS": "true", "GITHUB_REF": "refs/heads/" + branch}), \
                    patch.object(release_data, "gh") as gh, \
                    patch.object(release_data, "validate") as validate:
                with self.assertRaises(ValueError):
                    release_data.prepare(Path("unused"), channel)
                with self.assertRaises(ValueError):
                    release_data.publish(release_data.release_tag("test", channel), "a" * 64, channel=channel)
                gh.assert_not_called()
                validate.assert_not_called()

    def test_draft_lookup_uses_list_including_later_pages(self):
        entry = dict(tag_name="osm-peaks-test", draft=True, prerelease=False,
                     html_url="https://github.com/owner/repo/releases/tag/untagged-123")
        for target in ("osm-peaks-test", entry["html_url"]):
            with self.subTest(target=target), patch.object(release_data, "gh", return_value=json.dumps([[], [entry]])) as gh:
                self.assertEqual(entry, release_data.find_release(target))
                gh.assert_called_once_with("api", "--paginate", "--slurp", "repos/owner/repo/releases?per_page=100")

    def test_url_rejects_other_repository_and_channel(self):
        for target in ("https://example.com/owner/repo/releases/tag/osm-peaks-test",
                       "https://github.com/other/repo/releases/tag/osm-peaks-test",
                       "https://github.com/owner/repo/releases/tag/osm-peaks-test?x=1"):
            with self.subTest(target=target), patch.object(release_data, "releases") as entries:
                with self.assertRaises(ValueError):
                    release_data.find_release(target)
                entries.assert_not_called()
        url = "https://github.com/owner/repo/releases/tag/untagged-123"
        with patch.object(release_data, "releases", return_value=[dict(tag_name="osm-peaks-dev-test", html_url=url)]):
            with self.assertRaises(ValueError):
                release_data.find_release(url, "stable")

    def test_missing_duplicate_and_unavailable_release_stop(self):
        for entries in ([], [dict(tag_name="osm-peaks-test")] * 2):
            with patch.object(release_data, "releases", return_value=entries), self.assertRaises(ValueError):
                release_data.find_release("osm-peaks-test")
        with patch.object(release_data, "releases", side_effect=RuntimeError("HTTP 403")), self.assertRaises(RuntimeError):
            release_data.find_release("osm-peaks-test")

    def test_manual_url_uses_recorded_checksum_and_checks_assets(self):
        url = "https://github.com/owner/repo/releases/tag/untagged-123"
        for checksum in ("a" * 64, "b" * 64):
            entry = dict(tag_name="osm-peaks-test", html_url=url, draft=True, prerelease=False,
                         body=f"- SHA-256: `{checksum}`")
            with self.subTest(checksum=checksum), patch.object(release_data, "releases", return_value=[entry]), \
                    patch.object(release_data, "gh") as gh, \
                    patch.object(release_data, "fetch", return_value=self.current), \
                    patch.object(release_data, "previous_release", return_value=None), \
                    patch.object(release_data, "update_latest") as update:
                if checksum == "a" * 64:
                    release_data.publish(url, manual=True, reason="初回の件数を確認")
                    update.assert_called_once()
                else:
                    with self.assertRaises(ValueError):
                        release_data.publish(url, manual=True, reason="初回の件数を確認")
                    update.assert_not_called()
                    gh.assert_not_called()

    def test_missing_or_ambiguous_recorded_checksum_stops(self):
        line = "- SHA-256: `" + "a" * 64 + "`"
        for body in (None, "", line + "\n" + line, "- SHA-256: `invalid`"):
            with self.subTest(body=body), self.assertRaises(ValueError):
                release_data.reviewed_checksum(dict(body=body))

    def test_automatic_publish_still_requires_checksum(self):
        with patch.object(release_data, "gh") as gh, self.assertRaises(ValueError):
            release_data.publish("osm-peaks-test")
        gh.assert_not_called()

    def test_actions_branches_allow_only_their_channel(self):
        for branch, channel in (("main", "stable"), ("dev", "dev")):
            with self.subTest(branch=branch), \
                    patch.dict(os.environ, {"GITHUB_ACTIONS": "true", "GITHUB_REF": "refs/heads/" + branch}):
                release_data.check_branch(channel)
