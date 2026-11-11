"""GitHub を変更せずにDraft の整理条件と再確認を検証する。"""

from datetime import datetime, timezone
from io import BytesIO
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


def download_url(tag):
    return f"https://github.com/{cleanup.DATA_REPO}/releases/download/{tag}/manifest.json"


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

    def test_collect_references_includes_archived_app_manifests_and_both_pages_catalogs(self):
        app_url = cleanup.APP_PAGES + "osm-peaks/manifests/old.json"
        data_url = "https://shohei0205.github.io/yamamuki-data"
        data = cleanup.site.Catalog({}, {"points/old/history.json": [{"releaseUrl": download_url("history")}]})
        pointer = {"manifestUrl": app_url, "manifestSha256": "a" * 64}
        documents = {
            cleanup.APP_PAGES + "catalog.json": {
                "schemaVersion": 1, "datasets": {"osm-peaks": {"stable": pointer}}},
            cleanup.APP_PAGES + "osm-peaks/current.json": {"schemaVersion": 1, "stable": pointer},
            app_url: {"schemaVersion": 6, "downloadUrl": download_url("app-live")},
            data_url + "/points/osm-peaks-dev/manifest.json": {"schemaVersion": 6, "downloadUrl": download_url("direct")},
            data_url + "/points/osm-peaks/manifest.json": {"schemaVersion": 6, "downloadUrl": download_url("data-live")},
        }
        stable = {"schemaVersion": 2, "datasets": [{
            "id": "osm-peaks", "manifestUrl": data_url + "/points/osm-peaks/manifest.json",
            "manifest": {"downloadUrl": download_url("data-embedded")}}]}
        dev = {"schemaVersion": 2, "datasets": [{
            "id": "osm-peaks", "manifestUrl": data_url + "/points/osm-peaks-dev/manifest.json",
            "manifest": {"downloadUrl": download_url("dev-embedded")}}]}
        with (patch.dict(os.environ, GH_REPO=cleanup.DATA_REPO),
              patch.object(cleanup, "app_revision", return_value={"sha": "app-main"}),
              patch.object(cleanup, "app_source_documents", return_value=[{"downloadUrl": download_url(TAG)}]),
              patch.object(cleanup.site, "read_catalog", return_value=data),
              patch.object(cleanup.site, "read_points_catalog", side_effect=[stable, dev]),
              patch.object(cleanup, "read_json_url", side_effect=lambda url, digest=None: documents[url])):
            self.assertEqual(({TAG, "app-live", "data-live", "data-embedded", "dev-embedded", "direct", "history"},
                              "app-main"), cleanup.collect_references())

    def test_public_manifest_must_match_catalog_checksum(self):
        with patch.object(cleanup, "urlopen", return_value=BytesIO(b'{"schemaVersion":6}')):
            with self.assertRaisesRegex(ValueError, "SHA-256"):
                cleanup.read_json_url(cleanup.APP_PAGES + "osm-peaks/manifests/old.json", "a" * 64)

    def test_app_source_stops_on_truncated_tree(self):
        with patch.object(cleanup.site, "gh", return_value='{"truncated":true,"tree":[]}'):
            with self.assertRaisesRegex(ValueError, "最後まで"):
                list(cleanup.app_source_documents({"commit": {"tree": {"sha": "abc"}}}))

    def test_list_is_read_only_and_omits_referenced_candidates(self):
        with (patch.object(cleanup.site, "releases", return_value=[RELEASE, {**RELEASE, "tag_name": OTHER}]),
              patch.object(cleanup, "collect_references", return_value=({TAG}, "app-main")),
              patch.object(cleanup.site, "gh") as gh,
              patch.object(cleanup.site, "append_summary") as summary):
            cleanup.list_candidates(NOW)
        gh.assert_not_called()
        self.assertNotIn(TAG, summary.call_args.args[0])
        self.assertIn(OTHER, summary.call_args.args[0])

    def test_delete_rechecks_the_exact_release_after_loading_references(self):
        calls = []
        def collect():
            calls.append("refs")
            return set(), "app-main"

        def gh(*args):
            calls.append(args)
            return json.dumps(RELEASE)

        with (patch.dict(os.environ, GITHUB_ACTIONS="true", GITHUB_REF="refs/heads/main", GH_REPO=cleanup.DATA_REPO),
              patch.object(cleanup.site, "releases", return_value=[RELEASE]),
              patch.object(cleanup, "collect_references", side_effect=collect),
              patch.object(cleanup, "app_revision", return_value={"sha": "app-main"}),
              patch.object(cleanup.site, "gh", side_effect=gh),
              patch.object(cleanup.site, "append_summary")):
            cleanup.delete_candidate(TAG, NOW)
        self.assertEqual(["refs", ("api", f"repos/{cleanup.DATA_REPO}/releases/123"),
                          ("api", "--method", "DELETE", f"repos/{cleanup.DATA_REPO}/releases/123")], calls)

    def test_delete_stops_on_references_read_errors_or_a_changed_release(self):
        for protected, app_sha, remote, failure in (
                ({TAG}, "app-main", RELEASE, None),
                (set(), "new-app-main", RELEASE, None),
                (set(), "app-main", {**RELEASE, "draft": False, "prerelease": True}, None),
                (set(), "app-main", {**RELEASE, "tag_name": OTHER}, None),
                (set(), "app-main", {**RELEASE, "id": "123"}, None),
                (set(), "app-main", {**RELEASE, "body": ""}, None),
                (set(), "app-main", RELEASE, OSError("通信失敗"))):
            with (self.subTest(protected=protected, app_sha=app_sha, remote=remote, failure=failure),
                  patch.dict(os.environ, GITHUB_ACTIONS="true", GITHUB_REF="refs/heads/main", GH_REPO=cleanup.DATA_REPO),
                  patch.object(cleanup.site, "releases", return_value=[RELEASE]),
                  patch.object(cleanup, "collect_references", return_value=(protected, "app-main"), side_effect=failure),
                  patch.object(cleanup, "app_revision", return_value={"sha": app_sha}),
                  patch.object(cleanup.site, "gh", return_value=json.dumps(remote)) as gh):
                with self.assertRaises((ValueError, OSError)):
                    cleanup.delete_candidate(TAG, NOW)
                self.assertFalse(any("DELETE" in call.args for call in gh.call_args_list))

    def test_delete_rejects_forks_other_branches_and_multiple_tags_before_network(self):
        for repo, ref, tag in ((cleanup.DATA_REPO, "refs/heads/dev", TAG),
                               ("fork/yamamuki-data", "refs/heads/main", TAG),
                               (cleanup.DATA_REPO, "refs/heads/main", TAG + "\n" + OTHER)):
            with (self.subTest(repo=repo, ref=ref, tag=tag),
                  patch.dict(os.environ, GITHUB_ACTIONS="true", GITHUB_REF=ref, GH_REPO=repo),
                  patch.object(cleanup, "collect_references") as collect):
                with self.assertRaises(ValueError):
                    cleanup.delete_candidate(tag, NOW)
                collect.assert_not_called()
