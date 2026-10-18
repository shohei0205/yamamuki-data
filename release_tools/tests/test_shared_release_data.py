"""共通の公開処理が OSM の公開モジュールに依存しないことを確かめる。"""

import json
import subprocess
import sys
import os
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

from release_tools.publish_reviewed import ROOT
from release_tools import release_data


class SharedReleaseDataTests(unittest.TestCase):
    def test_publishers_do_not_import_osm_release_module(self):
        # 他のテストで読み込まれたモジュールの影響を避け、新しいプロセスで確かめる。
        result = subprocess.run([sys.executable, "-c", "\n".join([
            "import sys",
            "from release_tools import release_data, point_release, test_points",
            "assert 'scripts.release_data' not in sys.modules",
            "assert point_release.site is release_data",
            "assert test_points.release_data is release_data",
            "from importlib import import_module",
            "assert import_module('points.gsi-peaks.scripts.release_snapshot').release_data is release_data",
        ])], cwd=ROOT, capture_output=True, text=True)
        self.assertEqual(0, result.returncode, result.stderr)

    def test_shared_cli_runs_without_osm_working_directory(self):
        result = subprocess.run([sys.executable, "-m", "release_tools.release_data", "--help"],
                                cwd=ROOT, capture_output=True, text=True)
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("verify-pages", result.stdout)
        self.assertIn("remove-dataset", result.stdout)
        self.assertIn("retire-legacy", result.stdout)

    def test_any_dataset_verifies_every_attachment_before_publication(self):
        for corrupt in (False, True):
            with self.subTest(corrupt=corrupt), tempfile.TemporaryDirectory() as folder:
                directory = Path(folder)
                names = ("custom.json.gz", "manifest.json", "source-info.json")
                for name in names:
                    (directory / name).write_bytes(name.encode())
                notes = directory / "notes.md"
                notes.write_text("確認済み", encoding="utf-8")
                def gh(*args):
                    if args[:2] == ("release", "download"):
                        checked = Path(args[-1])
                        checked.mkdir()
                        for name in names:
                            shutil.copyfile(directory / name, checked / name)
                        if corrupt:
                            (checked / "source-info.json").write_bytes(b"corrupt")
                    return ""
                with patch.dict(os.environ, GITHUB_ACTIONS="false", GITHUB_SHA="abc"), \
                        patch.object(release_data, "gh", side_effect=gh) as calls:
                    if corrupt:
                        with self.assertRaisesRegex(ValueError, "一致しません"):
                            release_data.create_verified_release("custom-v1", directory, names, notes, "任意のデータ", "stable")
                    else:
                        release_data.create_verified_release("custom-v1", directory, names, notes, "任意のデータ", "stable")
                    edits = [call for call in calls.call_args_list if call.args[:2] == ("release", "edit")]
                    self.assertEqual(0 if corrupt else 1, len(edits))

    def test_creation_failure_does_not_download_or_publish(self):
        with tempfile.TemporaryDirectory() as folder:
            directory = Path(folder)
            (directory / "manifest.json").write_bytes(b"{}")
            with patch.dict(os.environ, GITHUB_ACTIONS="false", GITHUB_SHA="abc"), \
                    patch.object(release_data, "gh", side_effect=RuntimeError("既存タグ")) as calls:
                with self.assertRaises(RuntimeError):
                    release_data.create_verified_release("custom-v1", directory, ("manifest.json",), directory / "notes.md", "任意のデータ", "stable")
                self.assertEqual(1, calls.call_count)

    def test_retire_legacy_replaces_only_old_manifest(self):
        legacy = {"schemaVersion": 4, "version": "peaks-v1", "count": 10}
        current = {"schemaVersion": 5, "version": "20261007T071401Z-1-1", "downloadUrl": "https://example.com/osm-peaks.json.gz"}
        history = [{"kind": "snapshot", "version": "peaks-v1", "publishedAt": None,
                    "releaseUrl": "https://example.com/r", "downloadUrl": "https://example.com/d", "actionsRunUrl": None}]
        catalog = release_data.Catalog({"peaks/manifest.json": legacy, "points/osm-peaks/manifest.json": current},
                                       {"peaks/history.json": history})
        with tempfile.TemporaryDirectory() as folder:
            directory = Path(folder) / "pages"
            with patch.dict(os.environ, PAGES_DIRECTORY=str(directory), GITHUB_ACTIONS="false"), \
                    patch.object(release_data, "read_catalog", return_value=catalog), \
                    patch.object(release_data, "pages_url", return_value="https://example.com"):
                release_data.retire_legacy_manifest()
            self.assertEqual({"schemaVersion": 999}, json.loads((directory / "peaks/manifest.json").read_text(encoding="utf-8")))
            self.assertEqual(current, json.loads((directory / "points/osm-peaks/manifest.json").read_text(encoding="utf-8")))
            self.assertEqual(history, json.loads((directory / "peaks/history.json").read_text(encoding="utf-8"))["entries"])
            site = json.loads((directory / "catalog.json").read_text(encoding="utf-8"))
            self.assertEqual({"schemaVersion": 999}, site["manifests"]["peaks/manifest.json"])
        # 元のカタログは書き換えない。
        self.assertEqual(legacy, catalog["peaks/manifest.json"])

    def test_retire_legacy_stops_when_missing_or_already_retired(self):
        for manifests, message in (({}, "ありません"), ({"peaks/manifest.json": {"schemaVersion": 999}}, "置き換え済み")):
            with self.subTest(message=message), tempfile.TemporaryDirectory() as folder, \
                    patch.dict(os.environ, PAGES_DIRECTORY=str(Path(folder) / "pages"), GITHUB_ACTIONS="false"), \
                    patch.object(release_data, "read_catalog", return_value=release_data.Catalog(manifests)):
                with self.assertRaisesRegex(ValueError, message):
                    release_data.retire_legacy_manifest()
                self.assertFalse((Path(folder) / "pages").exists())

    def test_retire_legacy_runs_only_from_main(self):
        with patch.dict(os.environ, GITHUB_ACTIONS="true", GITHUB_REF="refs/heads/dev"), \
                patch.object(release_data, "read_catalog") as read:
            with self.assertRaisesRegex(ValueError, "main"):
                release_data.retire_legacy_manifest()
            read.assert_not_called()

    def test_compare_pages_allows_only_listed_changes(self):
        with tempfile.TemporaryDirectory() as folder:
            directory = Path(folder)
            (directory / "peaks").mkdir()
            (directory / "peaks/manifest.json").write_text('{"schemaVersion": 999}', encoding="utf-8")
            (directory / "catalog.json").write_text('{"a": 1}', encoding="utf-8")
            live = {"peaks/manifest.json": {"schemaVersion": 4}, "catalog.json": {"a": 1}}
            with patch.object(release_data, "read_pages_file", side_effect=live.get):
                release_data.compare_pages(directory, ["peaks/manifest.json"])
                with self.assertRaisesRegex(ValueError, "peaks/manifest.json"):
                    release_data.compare_pages(directory)
