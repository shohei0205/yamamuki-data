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
