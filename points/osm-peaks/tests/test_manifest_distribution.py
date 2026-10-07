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
        for channel, tag in (("stable", "osm-peaks-test"), ("dev", "osm-peaks-dev-test")):
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
        for channel, path in (("stable", "points/osm-peaks/manifest.json"), ("dev", "points/osm-peaks-dev/manifest.json")):
            old = {"points/osm-peaks/manifest.json": {"version": "old-stable"},
                   "points/osm-peaks-dev/manifest.json": {"version": "old-dev"},
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
            self.assertEqual(release_data.points_catalog(expected), json.loads((destination / "points/catalog.json").read_text(encoding="utf-8")))
            self.assertEqual(release_data.points_catalog(expected, "dev"), json.loads((destination / "points/catalog-dev.json").read_text(encoding="utf-8")))
            self.assertTrue((destination / ".nojekyll").exists())
            self.assertFalse(any(p.suffix == ".gz" for p in destination.rglob("*")))
            self.assertIn("pages_ready=true", (self.test_output / "output").read_text())

    def test_points_catalog_lists_all_points_channels_but_excludes_terrain(self):
        manifests = {"points/osm-peaks/manifest.json": {},
                     "points/osm-peaks-dev/manifest.json": {},
                     "points/curated_landmarks/manifest.json": {},
                     "terrain/manifest.json": {}, "peaks/manifest.json": {}}
        catalog = release_data.points_catalog(manifests)
        self.assertEqual(1, catalog["schemaVersion"])
        self.assertEqual(["curated_landmarks", "osm-peaks"],
                         [entry["id"] for entry in catalog["datasets"]])
        self.assertEqual("https://owner.github.io/repo/points/osm-peaks/manifest.json", catalog["datasets"][-1]["manifestUrl"])
        legacy = release_data.points_catalog({"points/osm-peaks/manifest.json": {"schemaVersion": 4, "version": "legacy"}})["datasets"][0]
        self.assertEqual(4, legacy["manifest"]["dataSchemaVersion"])
        unknown = release_data.points_catalog({"points/landmarks/manifest.json": {"schemaVersion": 5}})["datasets"][0]
        self.assertNotIn("dataSchemaVersion", unknown["manifest"])
        self.assertTrue(all("channel" not in entry for entry in catalog["datasets"]))
        self.assertEqual([], release_data.points_catalog({})["datasets"])
        development = release_data.points_catalog(manifests, "dev")
        self.assertEqual(["osm-peaks"], [entry["id"] for entry in development["datasets"]])
        self.assertTrue(all("channel" not in entry for entry in development["datasets"]))

    def test_points_catalog_names_use_manifest_and_preserve_other_data(self):
        for channel in ("stable", "dev"):
            suffix = "-dev" if channel == "dev" else ""
            manifests = {f"points/osm-peaks{suffix}/manifest.json": {},
                         f"points/landmarks{suffix}/manifest.json": {"name": "ランドマーク"},
                         f"points/custom{suffix}/manifest.json": {}}
            entries = release_data.points_catalog(manifests, channel)["datasets"]
            self.assertEqual({"osm-peaks": "山頂", "landmarks": "ランドマーク", "custom": "custom"},
                             {entry["id"]: entry["name"] for entry in entries})
            for value in (None, "", "  ", 1):
                with self.subTest(value=value), self.assertRaises(ValueError):
                    release_data.points_catalog({f"points/landmarks{suffix}/manifest.json": {"name": value}}, channel)

    def test_points_catalog_embeds_download_metadata_without_mutating_manifest(self):
        for channel in ("stable", "dev"):
            path = release_data.manifest_path(channel)
            manifest = dict(self.current[0], dataSchemaVersion=5, fileName="osm-peaks.json.gz", uncompressedSizeBytes=456,
                            downloadUrl="https://example.com/data.gz", pointCount=10,
                            license="ODbL-1.0", attribution="© OpenStreetMap contributors")
            entry = release_data.points_catalog({path: manifest}, channel)["datasets"][0]
            self.assertEqual(manifest, entry["manifest"])
            self.assertIsNot(manifest, entry["manifest"])
            del manifest["downloadUrl"]
            entry = release_data.points_catalog({path: manifest}, channel)["datasets"][0]
            self.assertEqual(release_data.download_url(manifest["version"], channel), entry["manifest"]["downloadUrl"])
            self.assertNotIn("downloadUrl", manifest)

    def test_publication_preserves_both_compatibility_channels(self):
        original = {"peaks/manifest.json": dict(self.current[0], version="old"),
                    "peaks-dev/manifest.json": dict(self.current[0], version="old-dev"),
                    "points/osm-peaks/manifest.json": dict(self.current[0], version="stable"),
                    "points/osm-peaks-dev/manifest.json": dict(self.current[0], version="dev")}
        histories = {key.replace("manifest.json", "history.json"): [] for key in original}
        destination = self.test_output / "cleanup"
        with patch.dict(os.environ, {"PAGES_DIRECTORY": str(destination)}), \
                patch.object(release_data, "read_catalog", return_value=release_data.Catalog(original, histories)):
            release_data.update_latest(self.test_output, self.current[0])
        document = json.loads((destination / "catalog.json").read_text(encoding="utf-8"))
        self.assertTrue((destination / "peaks/manifest.json").exists())
        self.assertTrue((destination / "peaks/history.json").exists())
        self.assertEqual("old", document["manifests"]["peaks/manifest.json"]["version"])
        self.assertTrue((destination / "peaks-dev/manifest.json").exists())
        self.assertTrue((destination / "peaks-dev/history.json").exists())
        self.assertEqual("old-dev", document["manifests"]["peaks-dev/manifest.json"]["version"])
        self.assertEqual({"peaks/manifest.json", "peaks-dev/manifest.json", "points/osm-peaks/manifest.json", "points/osm-peaks-dev/manifest.json"}, set(document["manifests"]))
        self.assertEqual(self.current[0], document["manifests"]["points/osm-peaks/manifest.json"])
        self.assertEqual("dev", document["manifests"]["points/osm-peaks-dev/manifest.json"]["version"])
        self.assertEqual({"peaks/history.json", "peaks-dev/history.json", "points/osm-peaks/history.json", "points/osm-peaks-dev/history.json"}, set(document["histories"]))
        with patch.object(release_data, "read_catalog", return_value={"peaks/manifest.json": original["peaks/manifest.json"]}):
            self.assertIsNone(release_data.read_manifest("stable"))

    def test_compatibility_stable_survives_each_channel_publication(self):
        for channel in ("stable", "dev"):
            compatibility = dict(self.current[0], version="compatibility")
            path = "peaks/manifest.json"
            history = [{"version": "compatibility", "kind": "publication"}]
            destination = self.test_output / ("compatibility-" + channel)
            with self.subTest(channel=channel), \
                    patch.dict(os.environ, {"PAGES_DIRECTORY": str(destination)}), \
                    patch.object(release_data, "read_catalog", return_value=release_data.Catalog({path: compatibility}, {"peaks/history.json": history})):
                release_data.update_latest(self.test_output, self.current[0], channel)
            self.assertEqual(compatibility, json.loads((destination / path).read_text(encoding="utf-8")))
            self.assertEqual(history, json.loads((destination / "peaks/history.json").read_text(encoding="utf-8"))["entries"])
            for filename in ("catalog.json", "catalog-dev.json"):
                catalog = json.loads((destination / "points" / filename).read_text(encoding="utf-8"))
                self.assertNotIn("peaks", [entry["id"] for entry in catalog["datasets"]])

    def test_verify_pages_waits_for_points_catalog(self):
        (self.test_output / "catalog.json").write_text('{"manifests": {}}', encoding="utf-8")
        (self.test_output / "points").mkdir()
        expected = {"schemaVersion": 1, "datasets": []}
        (self.test_output / "points/catalog.json").write_text(json.dumps(expected), encoding="utf-8")
        with patch.object(release_data, "read_catalog", return_value={}), \
                patch.object(release_data, "read_points_catalog", side_effect=[{}, expected]), \
                patch.object(release_data.time, "sleep") as sleep:
            release_data.verify_pages(self.test_output)
            sleep.assert_called_once_with(5)

    def test_verify_pages_checks_development_catalog_too(self):
        (self.test_output / "catalog.json").write_text('{"manifests": {}}', encoding="utf-8")
        (self.test_output / "points").mkdir()
        expected = {"schemaVersion": 1, "datasets": []}
        for filename in ("catalog.json", "catalog-dev.json"):
            (self.test_output / "points" / filename).write_text(json.dumps(expected), encoding="utf-8")
        with patch.object(release_data, "read_catalog", return_value={}), \
                patch.object(release_data, "read_points_catalog", side_effect=[expected, {}, expected, expected]) as read, \
                patch.object(release_data.time, "sleep") as sleep:
            release_data.verify_pages(self.test_output)
            self.assertEqual(["stable", "dev", "stable", "dev"], [call.args[0] for call in read.call_args_list])
            sleep.assert_called_once_with(5)

    def test_remove_dataset_preserves_other_channels_data_and_history(self):
        for channel in ("stable", "dev"):
            target = release_data.manifest_path(channel)
            other = release_data.manifest_path("dev" if channel == "stable" else "stable")
            legacy = ("peaks" if channel == "stable" else "peaks-dev") + "/manifest.json"
            manifests = {target: self.current[0], other: self.current[0], legacy: self.current[0],
                         "points/landmarks/manifest.json": self.current[0], "terrain/manifest.json": {"version": "terrain"}}
            histories = {path.replace("manifest.json", "history.json"): [] for path in manifests}
            destination = self.test_output / ("remove-" + channel)
            with patch.dict(os.environ, {"PAGES_DIRECTORY": str(destination)}), \
                    patch.object(release_data, "read_catalog", return_value=release_data.Catalog(manifests, histories)), \
                    patch.object(release_data, "gh") as gh:
                release_data.remove_dataset("osm-peaks", channel)
                gh.assert_not_called()
            document = json.loads((destination / "catalog.json").read_text(encoding="utf-8"))
            removed = {target}
            self.assertEqual(set(manifests) - removed, set(document["manifests"]))
            self.assertEqual(set(histories) - {path.replace("manifest.json", "history.json") for path in removed}, set(document["histories"]))
            self.assertEqual(release_data.points_catalog(document["manifests"], "dev"), json.loads((destination / "points/catalog-dev.json").read_text(encoding="utf-8")))
            self.assertFalse((destination / target).exists())
            self.assertTrue((destination / legacy).exists())
            self.assertEqual(release_data.points_catalog(document["manifests"]), json.loads((destination / "points/catalog.json").read_text(encoding="utf-8")))

    def test_remove_last_dataset_produces_empty_catalog(self):
        destination = self.test_output / "remove-last"
        with patch.dict(os.environ, {"PAGES_DIRECTORY": str(destination)}), \
                patch.object(release_data, "read_catalog", return_value={"points/landmarks/manifest.json": self.current[0]}):
            release_data.remove_dataset("landmarks", "stable")
        self.assertEqual([], json.loads((destination / "points/catalog.json").read_text(encoding="utf-8"))["datasets"])

    def test_remove_dataset_stops_on_missing_catalog_target_invalid_input_and_wrong_branch(self):
        for dataset in ("../osm-peaks", "osm-peaks-dev", "", "points/osm-peaks"):
            with self.subTest(dataset=dataset), patch.object(release_data, "read_catalog") as read:
                with self.assertRaises(ValueError):
                    release_data.remove_dataset(dataset, "stable")
                read.assert_not_called()
        for catalog in (None, {}):
            with patch.object(release_data, "read_catalog", return_value=catalog), patch.object(release_data, "write_site") as write:
                with self.assertRaises(ValueError):
                    release_data.remove_dataset("osm-peaks", "stable")
                write.assert_not_called()
        with patch.dict(os.environ, {"GITHUB_ACTIONS": "true", "GITHUB_REF": "refs/heads/main"}), \
                patch.object(release_data, "read_catalog") as read:
            with self.assertRaises(ValueError):
                release_data.remove_dataset("osm-peaks", "dev")
            read.assert_not_called()

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
        old = {"points/osm-peaks-dev/manifest.json": {"version": "previous-dev"}}
        with patch.dict(os.environ, {"PAGES_DIRECTORY": str(destination), "PAGES_INITIALIZE": "true"}), \
                patch.object(release_data, "read_catalog", return_value=old):
            release_data.update_latest(self.test_output, self.current[0])
        catalog = json.loads((destination / "catalog.json").read_text(encoding="utf-8"))
        self.assertEqual({"version": "previous-dev"}, catalog["manifests"]["points/osm-peaks-dev/manifest.json"])

    def test_read_catalog_validates_paths_and_schema(self):
        valid = {"schemaVersion": 1, "manifests": {"points/osm-peaks/manifest.json": self.current[0]}}
        documents = [valid, {}, {"schemaVersion": 2}, {"schemaVersion": 1, "manifests": []},
                     {"schemaVersion": 1, "manifests": {"../manifest.json": {}}},
                     {"schemaVersion": 1, "manifests": {"points/../manifest.json": {}}},
                     {"schemaVersion": 1, "manifests": {"points//osm-peaks/manifest.json": {}}},
                     {"schemaVersion": 1, "manifests": {"points/osm-peaks/manifest.json": []}}]
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
        catalog = {"points/osm-peaks/manifest.json": {"version": "stable"},
                   "points/osm-peaks-dev/manifest.json": {"version": "dev"},
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
        (self.test_output / "catalog.json").write_text(json.dumps({"manifests": {"points/osm-peaks/manifest.json": self.current[0]}}), encoding="utf-8")
        expected = {"points/osm-peaks/manifest.json": self.current[0]}
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
        original = {"points/osm-peaks/manifest.json": dict(self.current[0], version="newer"),
                    "points/osm-peaks-dev/manifest.json": dict(self.current[0], version="older-dev")}
        catalog = release_data.Catalog(original)
        for attempt in ("1", "2"):
            destination = self.test_output / ("history-" + attempt)
            with patch.dict(os.environ, {"PAGES_DIRECTORY": str(destination), "GITHUB_RUN_ID": "123", "GITHUB_RUN_ATTEMPT": attempt}), \
                    patch.object(release_data, "read_catalog", return_value=catalog):
                release_data.update_latest(self.test_output, self.current[0])
            document = json.loads((destination / "catalog.json").read_text(encoding="utf-8"))
            catalog = release_data.Catalog(document["manifests"], document["histories"])
            history = json.loads((destination / "points/osm-peaks/history.json").read_text(encoding="utf-8"))["entries"]
            self.assertEqual(int(attempt) + 1, len(history))
            self.assertEqual("newer", history[0]["version"])
            self.assertEqual("snapshot", history[0]["kind"])
            self.assertIsNone(history[0]["publishedAt"])
            self.assertIsNone(history[0]["actionsRunUrl"])
            self.assertEqual(self.current[0]["version"], history[-1]["version"])
            self.assertEqual("publication", history[-1]["kind"])
            self.assertTrue(history[-1]["publishedAt"].endswith("Z"))
            self.assertEqual("https://github.com/owner/repo/actions/runs/123/attempts/" + attempt, history[-1]["actionsRunUrl"])
            self.assertEqual("https://github.com/owner/repo/releases/tag/osm-peaks-test", history[-1]["releaseUrl"])
            self.assertEqual("https://github.com/owner/repo/releases/download/osm-peaks-test/osm-peaks.json.gz", history[-1]["downloadUrl"])
            dev = json.loads((destination / "points/osm-peaks-dev/history.json").read_text(encoding="utf-8"))["entries"]
            self.assertEqual(1, len(dev))
            self.assertEqual("older-dev", dev[0]["version"])

    def test_history_fetch_rejects_invalid_paths_and_contents(self):
        entry = release_data.history_entry(self.current[0], "dev")
        for histories in ([], {"../history.json": [entry]}, {"terrain/history.json": [entry]},
                          {"points/osm-peaks-dev/history.json": "invalid"}, {"points/osm-peaks-dev/history.json": [{}]}):
            document = {"schemaVersion": 1, "manifests": {"points/osm-peaks-dev/manifest.json": self.current[0]}, "histories": histories}
            with patch.object(release_data, "urlopen", return_value=io.BytesIO(json.dumps(document).encode())):
                with self.assertRaises(ValueError):
                    release_data.read_catalog()
        document["histories"] = {"points/osm-peaks-dev/history.json": [entry]}
        with patch.object(release_data, "urlopen", return_value=io.BytesIO(json.dumps(document).encode())):
            self.assertEqual(document["histories"], release_data.read_catalog().histories)

    def test_verify_pages_checks_history_as_well_as_manifest(self):
        manifests = {"points/osm-peaks/manifest.json": self.current[0]}
        histories = {"points/osm-peaks/history.json": [release_data.history_entry(self.current[0], "stable")]}
        (self.test_output / "catalog.json").write_text(json.dumps({"manifests": manifests, "histories": histories}), encoding="utf-8")
        with patch.object(release_data, "read_catalog", side_effect=[release_data.Catalog(manifests), release_data.Catalog(manifests, histories)]), \
                patch.object(release_data.time, "sleep") as sleep:
            release_data.verify_pages(self.test_output)
            sleep.assert_called_once_with(5)
