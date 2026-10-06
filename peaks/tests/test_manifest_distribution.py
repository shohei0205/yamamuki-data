"""ブランチを増やさず、他の配布先を維持して Pages の manifest を作る。"""

import io
import json
import os
from pathlib import Path
import unittest
from unittest.mock import patch
from urllib.error import HTTPError, URLError

from scripts import release_data
import test_release_data as baseline


class ManifestDistributionTests(unittest.TestCase):
    setUp = baseline.ReleaseDataTests.setUp

    def test_baseline_absent_and_unavailable(self):
        with patch.object(release_data, "read_manifest", return_value=None):
            self.assertIsNone(release_data.previous_release(Path("unused")))
        with patch.object(release_data, "read_manifest", side_effect=RuntimeError("HTTP 403")):
            with self.assertRaises(RuntimeError):
                release_data.previous_release(Path("unused"))

    def test_baseline_requires_matching_published_release(self):
        for channel, tag in (("stable", "peaks-test"), ("dev", "peaks-dev-test")):
            entry = dict(tag_name=tag, draft=False, prerelease=channel == "dev")
            with self.subTest(channel=channel), \
                    patch.object(release_data, "read_manifest", return_value=self.current[0]) as read, \
                    patch.object(release_data, "releases", return_value=[entry]), \
                    patch.object(release_data, "fetch", return_value=self.current) as fetch:
                self.assertEqual(self.current, release_data.previous_release(Path("unused"), channel))
                read.assert_called_once_with(channel)
                self.assertEqual(tag, fetch.call_args.args[0])
                for changes in ({"draft": True}, {"prerelease": channel != "dev"}, {"tag_name": "terrain-test"}):
                    with patch.object(release_data, "releases", return_value=[dict(entry, **changes)]):
                        with self.assertRaises(ValueError):
                            release_data.previous_release(Path("unused"), channel)
                with patch.object(release_data, "fetch", return_value=(dict(self.current[0], sha256="b" * 64), [])):
                    with self.assertRaises(ValueError):
                        release_data.previous_release(Path("unused"), channel)

    def test_site_preserves_other_channels_and_data_types(self):
        for channel, path in (("stable", "peaks/manifest.json"), ("dev", "peaks-dev/manifest.json")):
            old = {"peaks/manifest.json": {"version": "old-stable"},
                   "peaks-dev/manifest.json": {"version": "old-dev"},
                   "terrain/manifest.json": {"version": "terrain"}}
            destination = self.test_output / channel
            with patch.dict(os.environ, {"PAGES_DIRECTORY": str(destination)}), \
                    patch.object(release_data, "read_catalog", return_value=dict(old)), \
                    patch.object(release_data, "gh") as gh:
                release_data.update_latest(self.test_output, self.current[0], channel)
                gh.assert_not_called()
            expected = dict(old, **{path: self.current[0]})
            for target, manifest in expected.items():
                self.assertEqual(manifest, json.loads((destination / target).read_text(encoding="utf-8")))
            catalog = json.loads((destination / "catalog.json").read_text(encoding="utf-8"))
            self.assertEqual(expected, catalog["manifests"])
            self.assertTrue((destination / ".nojekyll").exists())
            self.assertFalse(any(p.suffix == ".gz" for p in destination.rglob("*")))
            self.assertIn("pages_ready=true", (self.test_output / "output").read_text())

    def test_initialization_requires_explicit_manual_setting(self):
        destination = self.test_output / "pages"
        with patch.dict(os.environ, {"PAGES_DIRECTORY": str(destination), "PAGES_INITIALIZE": "false"}), \
                patch.object(release_data, "read_catalog", return_value=None):
            with self.assertRaises(ValueError):
                release_data.update_latest(self.test_output, self.current[0])
            self.assertFalse(destination.exists())

    def test_first_migration_does_not_read_old_releases(self):
        for channel in ("stable", "dev"):
            destination = self.test_output / channel
            with patch.dict(os.environ, {"PAGES_DIRECTORY": str(destination), "PAGES_INITIALIZE": "true"}), \
                    patch.object(release_data, "read_catalog", return_value=None), \
                    patch.object(release_data, "gh") as gh:
                release_data.update_latest(self.test_output, self.current[0], channel)
                gh.assert_not_called()
            catalog = json.loads((destination / "catalog.json").read_text(encoding="utf-8"))
            self.assertEqual({release_data.manifest_path(channel): self.current[0]}, catalog["manifests"])

    def test_initialization_setting_does_not_reset_existing_catalog(self):
        destination = self.test_output / "pages"
        old = {"peaks-dev/manifest.json": {"version": "previous-dev"}}
        with patch.dict(os.environ, {"PAGES_DIRECTORY": str(destination), "PAGES_INITIALIZE": "true"}), \
                patch.object(release_data, "read_catalog", return_value=old):
            release_data.update_latest(self.test_output, self.current[0])
        catalog = json.loads((destination / "catalog.json").read_text(encoding="utf-8"))
        self.assertEqual({"version": "previous-dev"}, catalog["manifests"]["peaks-dev/manifest.json"])

    def test_read_catalog_validates_paths_and_schema(self):
        valid = {"schemaVersion": 1, "manifests": {"peaks/manifest.json": self.current[0]}}
        documents = [valid, {}, {"schemaVersion": 2}, {"schemaVersion": 1, "manifests": []},
                     {"schemaVersion": 1, "manifests": {"../manifest.json": {}}},
                     {"schemaVersion": 1, "manifests": {"peaks/manifest.json": []}}]
        for document in documents:
            with self.subTest(document=document), \
                    patch.object(release_data, "urlopen", return_value=io.BytesIO(json.dumps(document).encode())) as read:
                if document == valid:
                    self.assertEqual(valid["manifests"], release_data.read_catalog())
                    self.assertTrue(read.call_args.args[0].full_url.startswith("https://owner.github.io/repo/catalog.json?"))
                    self.assertEqual(60, read.call_args.kwargs["timeout"])
                else:
                    with self.assertRaises(ValueError):
                        release_data.read_catalog()

    def test_only_404_can_be_uninitialized(self):
        for code in (404, 403, 500):
            with patch.object(release_data, "urlopen", side_effect=HTTPError("url", code, "error", {}, None)):
                if code == 404:
                    self.assertIsNone(release_data.read_catalog())
                else:
                    with self.assertRaises(HTTPError):
                        release_data.read_catalog()
        with patch.object(release_data, "urlopen", side_effect=URLError("通信失敗")):
            with self.assertRaises(URLError):
                release_data.read_catalog()

    def test_read_selected_channel_without_old_release_fallback(self):
        catalog = {"peaks/manifest.json": {"version": "stable"},
                   "peaks-dev/manifest.json": {"version": "dev"},
                   "terrain/manifest.json": {"version": "terrain"}}
        for channel in ("stable", "dev"):
            with patch.object(release_data, "read_catalog", return_value=catalog), \
                    patch.object(release_data, "gh") as gh:
                self.assertEqual(channel, release_data.read_manifest(channel)["version"])
                gh.assert_not_called()
            with patch.object(release_data, "read_catalog", return_value=None), \
                    patch.object(release_data, "gh") as gh:
                self.assertIsNone(release_data.read_manifest(channel))
                gh.assert_not_called()

    def test_fetch_failure_never_outputs_site_or_ready_flag(self):
        destination = self.test_output / "pages"
        with patch.dict(os.environ, {"PAGES_DIRECTORY": str(destination)}), \
                patch.object(release_data, "read_catalog", side_effect=URLError("通信失敗")):
            with self.assertRaises(URLError):
                release_data.update_latest(self.test_output, self.current[0])
            self.assertFalse(destination.exists())
            self.assertFalse((self.test_output / "output").exists())

    def test_verify_pages_waits_for_exact_catalog(self):
        (self.test_output / "catalog.json").write_text(json.dumps({"manifests": {"peaks/manifest.json": self.current[0]}}), encoding="utf-8")
        expected = {"peaks/manifest.json": self.current[0]}
        with patch.object(release_data, "read_catalog", side_effect=[None, {}, expected]), patch.object(release_data.time, "sleep") as sleep:
            release_data.verify_pages(self.test_output)
            self.assertEqual(2, sleep.call_count)

    def test_verify_pages_failure_is_reported(self):
        (self.test_output / "catalog.json").write_text('{"manifests": {}}', encoding="utf-8")
        with patch.object(release_data, "read_catalog", return_value=None) as read, patch.object(release_data.time, "sleep"):
            with self.assertRaises(RuntimeError):
                release_data.verify_pages(self.test_output)
            self.assertEqual(12, read.call_count)


    def test_history_records_republication_and_rollback_without_losing_other_channel(self):
        original = {"peaks/manifest.json": dict(self.current[0], version="newer"),
                    "peaks-dev/manifest.json": dict(self.current[0], version="older-dev")}
        catalog = release_data.Catalog(original)
        for attempt in ("1", "2"):
            destination = self.test_output / ("history-" + attempt)
            with patch.dict(os.environ, {"PAGES_DIRECTORY": str(destination), "GITHUB_RUN_ID": "123", "GITHUB_RUN_ATTEMPT": attempt}), \
                    patch.object(release_data, "read_catalog", return_value=catalog):
                release_data.update_latest(self.test_output, self.current[0])
            document = json.loads((destination / "catalog.json").read_text(encoding="utf-8"))
            catalog = release_data.Catalog(document["manifests"], document["histories"])
            history = json.loads((destination / "peaks/history.json").read_text(encoding="utf-8"))["entries"]
            self.assertEqual(int(attempt) + 1, len(history))
            self.assertEqual("newer", history[0]["version"])
            self.assertEqual("snapshot", history[0]["kind"])
            self.assertIsNone(history[0]["publishedAt"])
            self.assertIsNone(history[0]["actionsRunUrl"])
            self.assertEqual(self.current[0]["version"], history[-1]["version"])
            self.assertEqual("publication", history[-1]["kind"])
            self.assertTrue(history[-1]["publishedAt"].endswith("Z"))
            self.assertEqual("https://github.com/owner/repo/actions/runs/123/attempts/" + attempt, history[-1]["actionsRunUrl"])
            self.assertEqual("https://github.com/owner/repo/releases/tag/peaks-test", history[-1]["releaseUrl"])
            self.assertEqual("https://github.com/owner/repo/releases/download/peaks-test/japan-mountains.json.gz", history[-1]["downloadUrl"])
            dev = json.loads((destination / "peaks-dev/history.json").read_text(encoding="utf-8"))["entries"]
            self.assertEqual(1, len(dev))
            self.assertEqual("older-dev", dev[0]["version"])

    def test_history_fetch_rejects_invalid_paths_and_contents(self):
        entry = release_data.history_entry(self.current[0], "dev")
        for histories in ([], {"../history.json": [entry]}, {"terrain/history.json": [entry]},
                          {"peaks-dev/history.json": "invalid"}, {"peaks-dev/history.json": [{}]}):
            document = {"schemaVersion": 1, "manifests": {"peaks-dev/manifest.json": self.current[0]}, "histories": histories}
            with patch.object(release_data, "urlopen", return_value=io.BytesIO(json.dumps(document).encode())):
                with self.assertRaises(ValueError):
                    release_data.read_catalog()
        document["histories"] = {"peaks-dev/history.json": [entry]}
        with patch.object(release_data, "urlopen", return_value=io.BytesIO(json.dumps(document).encode())):
            self.assertEqual(document["histories"], release_data.read_catalog().histories)

    def test_verify_pages_checks_history_as_well_as_manifest(self):
        manifests = {"peaks/manifest.json": self.current[0]}
        histories = {"peaks/history.json": [release_data.history_entry(self.current[0], "stable")]}
        (self.test_output / "catalog.json").write_text(json.dumps({"manifests": manifests, "histories": histories}), encoding="utf-8")
        with patch.object(release_data, "read_catalog", side_effect=[release_data.Catalog(manifests), release_data.Catalog(manifests, histories)]), \
                patch.object(release_data.time, "sleep") as sleep:
            release_data.verify_pages(self.test_output)
            sleep.assert_called_once_with(5)
