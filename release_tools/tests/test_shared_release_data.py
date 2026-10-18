"""共通の公開処理が OSM の公開モジュールに依存しないことを確かめる。"""

import subprocess
import sys
import unittest

from release_tools.publish_reviewed import ROOT


class SharedReleaseDataTests(unittest.TestCase):
    def test_publishers_do_not_import_osm_release_module(self):
        # 他のテストで読み込まれたモジュールの影響を避け、新しいプロセスで確かめる。
        result = subprocess.run([sys.executable, "-c", "\n".join([
            "import sys",
            "from release_tools import release_data, point_release, gsi_peaks, test_points",
            "assert 'scripts.release_data' not in sys.modules",
            "assert point_release.site is release_data",
            "assert test_points.release_data is release_data",
            "assert gsi_peaks.release_data is release_data",
        ])], cwd=ROOT, capture_output=True, text=True)
        self.assertEqual(0, result.returncode, result.stderr)

    def test_shared_cli_runs_without_osm_working_directory(self):
        result = subprocess.run([sys.executable, "-m", "release_tools.release_data", "--help"],
                                cwd=ROOT, capture_output=True, text=True)
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("verify-pages", result.stdout)
        self.assertIn("remove-dataset", result.stdout)
