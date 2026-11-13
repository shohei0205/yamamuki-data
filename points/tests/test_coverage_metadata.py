"""GeoJSON・言語の共通検証と、通常・逐次・配布用の読み書きを確かめる。"""

from copy import deepcopy
import gzip
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "points"))
sys.path.insert(0, str(ROOT / "points/osm-peaks"))
from coverage_metadata import validate_coverage, validate_language
from document_metadata import copy_manifest_metadata, validate_metadata
from all_points_io import read_points, write_points
from scripts.point_document import document, rows
from scripts.supplements import supplement_document, validate_supplements
from scripts.build_data import write_distribution
from scripts.distribution_parts import write_parts, validate_parts

CASES = json.loads((ROOT / "schemas/testdata/coverage-metadata.json").read_bytes())


class CoverageTests(unittest.TestCase):
    def test_nonfinite_and_large_coordinates_are_rejected(self):
        for value in [float("nan"), float("inf"), -float("inf"), 10 ** 1000]:
            geometry = {"type": "Polygon", "coordinates": [[[value, 30], [140, 30], [140, 40], [value, 30]]]}
            with self.subTest(value=str(value)), self.assertRaises(ValueError):
                validate_coverage({"geometry": geometry})

    def test_cli_rejects_null_coverage_before_reading_pbf(self):
        from scripts import build_data
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "coverage.json"
            path.write_text("null", encoding="utf-8")
            with patch.object(sys, "argv", ["build_data", "missing.pbf", "--version", "test", "--coverage", str(path)]), patch.object(build_data, "build") as build:
                with self.assertRaises(ValueError):
                    build_data.main()
                build.assert_not_called()

    def test_shared_cases(self):
        for case in CASES:
            with self.subTest(name=case["name"]):
                def check():
                    info = case["metadata"]
                    if "coverage" in info:
                        validate_coverage(info["coverage"], surface=case["surface"])
                    if "language" in info:
                        validate_language(info["language"])
                if case["valid"]:
                    check()
                else:
                    with self.assertRaises(ValueError):
                        check()

    def test_invalid_points_metadata_is_rejected_by_every_path(self):
        point = {"id": "osm:1", "name": "山", "latitude": 35, "longitude": 139}
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "input.json"
            for case in CASES:
                if case["surface"] or case["valid"]:
                    continue
                info = case["metadata"]
                body = {"schemaFormat": "yamamuki-points", "schemaVersion": 6,
                        "metadata": info, "points": [point]}
                path.write_text(json.dumps(body), encoding="utf-8")
                with self.subTest(name=case["name"]):
                    for check in (lambda: validate_metadata(info), lambda: rows(body, 6),
                                  lambda: list(read_points(path, chunk_size=7)),
                                  lambda: write_points([point], folder, "output", metadata=info),
                                  lambda: supplement_document({"schemaVersion": 2,
                                      "schemaFormat": "yamamuki-point-supplements", "metadata": info, "points": []})):
                        with self.assertRaises(ValueError):
                            check()

    def test_geometry_and_language_survive_stream_and_manifest_copy(self):
        info = {"coverage": next(c["metadata"]["coverage"] for c in CASES if c["name"] == "穴"),
                "language": "ja", "future": {"nested": [1]}}
        before = deepcopy(info)
        point = {"id": "osm:1", "name": "山", "latitude": 35, "longitude": 139}
        with tempfile.TemporaryDirectory() as folder:
            write_points([point], folder, "output", metadata=info)
            metadata = {}
            self.assertEqual(list(read_points(Path(folder) / "output.json.gz", chunk_size=3, metadata=metadata)), [point])
            for key, value in info.items():
                self.assertEqual(metadata[key], value)
            doc = document([point], **info)
            manifest = {"schemaVersion": 6, "coverage": {}, "language": "en"}
            copy_manifest_metadata(manifest, doc["metadata"])
            self.assertEqual(manifest["metadata"], doc["metadata"])
            self.assertNotIn("coverage", manifest)
            self.assertNotIn("language", manifest)
        self.assertEqual(info, before)

    def test_osm_distribution_and_source_keep_coverage(self):
        coverage = next(c["metadata"]["coverage"] for c in CASES if c["name"] == "離れた領域")
        points = [{"id": "osm:1", "name": "山", "latitude": 35, "longitude": 139}]
        supplements = {"schemaVersion": 1, "points": []}
        with tempfile.TemporaryDirectory() as folder:
            manifest = write_distribution(points, folder, "test", "2026-11-13T00:00:00Z",
                                          "2026-11-12T00:00:00Z", coverage=coverage)
            self.assertEqual(manifest["metadata"]["coverage"], coverage)
            self.assertEqual(manifest["metadata"]["language"], "ja")
            body = json.loads(gzip.decompress((Path(folder) / "osm-peaks.json.gz").read_bytes()))
            self.assertEqual(body["metadata"], manifest["metadata"])
            write_parts(folder, points, supplements, manifest)
            validate_parts(folder, manifest, points)
            source = json.loads(gzip.decompress((Path(folder) / "osm-peaks-source.json.gz").read_bytes()))
            self.assertEqual(source["metadata"]["coverage"], coverage)
            self.assertEqual(source["metadata"]["language"], "ja")

    def test_supplements_preserve_coverage_and_language(self):
        info = {"coverage": {"regions": ["JP"]}, "language": "ja"}
        supplement = {"schemaFormat": "yamamuki-point-supplements", "schemaVersion": 2,
                      "metadata": info, "points": []}
        restored = supplement_document(validate_supplements(supplement))
        self.assertEqual(restored["metadata"]["coverage"], info["coverage"])
        self.assertEqual(restored["metadata"]["language"], "ja")
