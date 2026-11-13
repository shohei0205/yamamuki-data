"""PBF の境界記録、追加抽出範囲、取得失敗を確認する。"""

import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from coverage_fixture import POLY, save_boundary
from scripts.source_coverage import capture_boundary, generated_coverage
from scripts.download_source import save_source_info
from shapely.geometry import shape, Point


class SourceCoverageTests(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.pbf = Path(folder.name) / "japan.osm.pbf"
        self.pbf.write_bytes(b"PBF fixture")
        self.source = {"url": "https://download.geofabrik.de/asia/japan-261113.osm.pbf",
                       "md5": hashlib.md5(self.pbf.read_bytes()).hexdigest(), "sizeBytes": self.pbf.stat().st_size}
        save_source_info(self.pbf, self.source)

    def test_capture_reuses_and_checks_the_boundary_paired_with_pbf(self):
        with patch("scripts.source_coverage.request", return_value=io.BytesIO(POLY)) as request:
            geometry, record = capture_boundary(self.pbf)
        self.assertEqual(record["sha256"], hashlib.sha256(POLY).hexdigest())
        self.assertEqual(request.call_args.args[0], "https://download.geofabrik.de/asia/japan.poly")
        save_source_info(self.pbf, self.source)
        with patch("scripts.source_coverage.request", side_effect=AssertionError("再取得した")):
            self.assertEqual(capture_boundary(self.pbf), (geometry, record))
        Path(str(self.pbf) + ".poly").write_bytes(b"broken")
        with self.assertRaises(ValueError):
            capture_boundary(self.pbf)

    def test_only_selected_northern_regions_are_added(self):
        save_boundary(self.pbf)
        coverage, _ = generated_coverage(self.pbf, additional=True)
        area = shape(coverage["geometry"])
        self.assertTrue(area.covers(Point(147.87, 45.03)))
        self.assertFalse(area.covers(Point(150, 50)))
        self.assertFalse(shape(generated_coverage(self.pbf)[0]["geometry"]).covers(Point(147.87, 45.03)))
        self.assertEqual(coverage["regions"], ["JP"])

    def test_failed_or_invalid_download_does_not_replace_record(self):
        before = Path(str(self.pbf) + ".source.json").read_bytes()
        for error in [OSError("取得失敗"), None]:
            with patch("scripts.source_coverage.request", side_effect=error,
                       return_value=io.BytesIO(b"not a polygon")), self.assertRaises((ValueError, OSError)):
                capture_boundary(self.pbf)
            self.assertEqual(Path(str(self.pbf) + ".source.json").read_bytes(), before)
            self.assertFalse(Path(str(self.pbf) + ".poly").exists())
