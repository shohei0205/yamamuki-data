"""復元ブランチの制限と現行データの保持を確認する。"""

import json
import os
from pathlib import Path
import unittest
from unittest.mock import patch

from scripts import release_channels, release_data
import test_release_data as baseline


class RestorePeaksTests(unittest.TestCase):
    setUp = baseline.ReleaseDataTests.setUp

    def test_recovery_branch_can_publish_only_stable(self):
        with patch.dict(os.environ, {"GITHUB_ACTIONS": "true", "GITHUB_REF": "refs/heads/codex/restore-peaks-stable"}):
            release_channels.check_branch("stable")
            with self.assertRaises(ValueError):
                release_channels.check_branch("dev")
        with patch.dict(os.environ, {"GITHUB_ACTIONS": "true", "GITHUB_REF": "refs/heads/codex/unrelated"}):
            with self.assertRaises(ValueError):
                release_channels.check_branch("stable")

    def test_restoration_preserves_point_catalogs_and_histories(self):
        manifest = dict(self.current[0], schemaVersion=5, dataSchemaVersion=5, name="山頂",
                        downloadUrl="https://github.com/owner/repo/releases/download/osm-peaks-dev-v1/osm-peaks.json.gz")
        manifests = {"points/osm-peaks-dev/manifest.json": manifest}
        histories = {"points/osm-peaks-dev/history.json": []}
        destination = self.test_output / "restore"
        with patch.dict(os.environ, {"PAGES_DIRECTORY": str(destination)}), \
                patch.object(release_data, "read_catalog", return_value=release_data.Catalog(manifests, histories)):
            release_data.update_latest(self.test_output, self.current[0], "stable")
        document = json.loads((destination / "catalog.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest, document["manifests"]["points/osm-peaks-dev/manifest.json"])
        self.assertEqual([], document["histories"]["points/osm-peaks-dev/history.json"])
        self.assertTrue((destination / "peaks/manifest.json").exists())
        self.assertTrue((destination / "peaks/history.json").exists())
        self.assertEqual([], json.loads((destination / "points/catalog.json").read_text(encoding="utf-8"))["datasets"])
        entries = json.loads((destination / "points/catalog-dev.json").read_text(encoding="utf-8"))["datasets"]
        self.assertEqual(["osm-peaks"], [entry["id"] for entry in entries])
        self.assertEqual(manifest, entries[0]["manifest"])
