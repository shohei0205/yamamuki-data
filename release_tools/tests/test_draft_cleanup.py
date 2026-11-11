"""GitHub を変更せずに Draft の整理条件と再確認を検証する。"""

from datetime import datetime, timezone
import json
import os
import unittest
from unittest.mock import patch

from release_tools import draft_cleanup as cleanup


TAG = "osm-peaks-20260930T000000Z-123-1"
OTHER = "gsi-peaks-20260930T000000Z-123-1"
NOW = datetime(2026, 11, 10, tzinfo=timezone.utc)
RELEASE = {
    "id": 123,
    "tag_name": TAG,
    "draft": True,
    "prerelease": False,
    "published_at": None,
    "created_at": "2026-09-30T00:00:00Z",
    "body": cleanup.site.TEMPORARY_DRAFT_MARKER,
}



class DraftCleanupTests(unittest.TestCase):
    def test_only_old_never_published_drafts_are_candidates(self):
        self.assertEqual("2026-09-30", str(cleanup.draft_date(RELEASE, NOW)))
        self.assertIsNotNone(cleanup.draft_date({**RELEASE, "created_at": "2026-11-03T00:00:00Z"}, NOW))
        for changes in ({"body": ""}, {"draft": False}, {"published_at": "2026-09-30T00:00:00Z"},
                        {"created_at": "2026-11-04T00:00:00Z"},
                        {"created_at": "invalid"}, {"created_at": None},
                        {"created_at": "2026-09-30T00:00:00"},
                        {"tag_name": TAG.replace("osm-peaks-", "osm-peaks-dev-")},
                        {"tag_name": "viewer-v1.0.0-dev.123"}):
            with self.subTest(changes=changes):
                self.assertIsNone(cleanup.draft_date({**RELEASE, **changes}, NOW))

    def test_weekly_cleanup_selects_only_marked_old_drafts(self):
        releases = [RELEASE, {**RELEASE, "tag_name": OTHER, "body": ""},
                    {**RELEASE, "draft": False, "prerelease": True},
                    {**RELEASE, "created_at": "2099-01-01T00:00:00Z"}]
        with (patch.object(cleanup.site, "releases", return_value=releases),
              patch.object(cleanup, "delete_candidate") as delete,
              patch("sys.argv", ["draft_cleanup", "delete-old"])):
            cleanup.main()
        self.assertEqual([TAG], [call.args[0] for call in delete.call_args_list])

    def test_list_is_read_only_and_includes_all_eligible_drafts(self):
        with (patch.object(cleanup.site, "releases", return_value=[RELEASE, {**RELEASE, "tag_name": OTHER}]),
              patch.object(cleanup.site, "gh") as gh,
              patch.object(cleanup.site, "append_summary") as summary):
            cleanup.list_candidates(NOW)
        gh.assert_not_called()
        self.assertIn(TAG, summary.call_args.args[0])
        self.assertIn(OTHER, summary.call_args.args[0])

    def test_delete_rechecks_the_exact_release_immediately_before_deletion(self):
        calls = []
        def gh(*args):
            calls.append(args)
            return json.dumps(RELEASE)

        with (patch.dict(os.environ, GITHUB_ACTIONS="true", GITHUB_REF="refs/heads/main", GH_REPO=cleanup.DATA_REPO),
              patch.object(cleanup.site, "releases", return_value=[RELEASE]),
              patch.object(cleanup.site, "gh", side_effect=gh),
              patch.object(cleanup.site, "append_summary")):
            cleanup.delete_candidate(TAG, NOW)
        self.assertEqual([("api", f"repos/{cleanup.DATA_REPO}/releases/123"),
                          ("api", "--method", "DELETE", f"repos/{cleanup.DATA_REPO}/releases/123")], calls)

    def test_delete_stops_on_read_errors_or_a_changed_release(self):
        for remote, failure in (
                ({**RELEASE, "draft": False, "prerelease": True}, None),
                ({**RELEASE, "published_at": "2026-11-01T00:00:00Z"}, None),
                ({**RELEASE, "tag_name": OTHER}, None),
                ({**RELEASE, "id": "123"}, None),
                ({**RELEASE, "body": ""}, None),
                ({**RELEASE, "created_at": "2026-11-09T00:00:00Z"}, None),
                (RELEASE, OSError("通信失敗"))):
            with (self.subTest(remote=remote, failure=failure),
                  patch.dict(os.environ, GITHUB_ACTIONS="true", GITHUB_REF="refs/heads/main", GH_REPO=cleanup.DATA_REPO),
                  patch.object(cleanup.site, "releases", return_value=[RELEASE]),
                  patch.object(cleanup.site, "gh", return_value=json.dumps(remote), side_effect=failure) as gh):
                with self.assertRaises((ValueError, OSError)):
                    cleanup.delete_candidate(TAG, NOW)
                self.assertFalse(any("DELETE" in call.args for call in gh.call_args_list))

    def test_delete_rejects_forks_other_branches_and_multiple_tags_before_network(self):
        for repo, ref, tag in ((cleanup.DATA_REPO, "refs/heads/dev", TAG),
                               ("fork/yamamuki-data", "refs/heads/main", TAG),
                               (cleanup.DATA_REPO, "refs/heads/main", TAG + "\n" + OTHER)):
            with (self.subTest(repo=repo, ref=ref, tag=tag),
                  patch.dict(os.environ, GITHUB_ACTIONS="true", GITHUB_REF=ref, GH_REPO=repo),
                  patch.object(cleanup.site, "releases") as releases):
                with self.assertRaises(ValueError):
                    cleanup.delete_candidate(tag, NOW)
                releases.assert_not_called()
