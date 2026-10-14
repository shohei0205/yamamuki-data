"""大きな地点配列の逐次読み込みと、生成失敗時の元ファイル保持を確認する。"""

import gzip
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from all_points_io import read_points, write_points


class AllPointsIoTests(unittest.TestCase):
    def test_json_chunks_bom_and_gzip(self):
        points = [{"id": "1", "name": "山、[麓]", "latitude": 35, "longitude": 139},
                  {"id": "2", "name": "駅", "latitude": 36, "longitude": 140}]
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "points.json"
            for indent in (None, 2):
                path.write_text(json.dumps(points, ensure_ascii=False, indent=indent), encoding="utf-8-sig")
                self.assertEqual(list(read_points(path, chunk_size=3)), points)
            archive = path.with_suffix(".json.gz")
            archive.write_bytes(gzip.compress(path.read_bytes()))
            self.assertEqual(list(read_points(archive, chunk_size=1)), points)

    def test_rejects_truncated_or_non_array_json(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "bad.json"
            for text in ('{}', '[{}', '[{},]', '[{}] trailing', '[42]', '[{} {}]'):
                with self.subTest(text=text):
                    path.write_text(text, encoding="utf-8")
                    with self.assertRaises(ValueError):
                        list(read_points(path, chunk_size=2))

    def test_output_validation_preserves_previous_files(self):
        point = {"id": "1", "name": "山", "latitude": 35, "longitude": 139,
                 "elevationM": 0, "type": "peak", "provenance": {"dataSource": "osm"}}
        with tempfile.TemporaryDirectory() as folder:
            report = write_points([point], folder, "points")
            path = Path(folder) / "points.json"
            raw = path.read_bytes()
            archive = path.with_suffix(".json.gz")
            self.assertEqual(gzip.decompress(archive.read_bytes()), raw)
            self.assertEqual(report["elevationCount"], 1)
            for bad in ([point, point], [{**point, "latitude": float('nan')}],
                        [{**point, "elevationM": float('inf')}], [{**point, "provenance": []}]):
                with self.assertRaises(ValueError):
                    write_points(bad, folder, "points")
                self.assertEqual(path.read_bytes(), raw)
            self.assertFalse(list(Path(folder).glob('*.part')))
            self.assertEqual(write_points([point], folder, "points")["sha256"], report["sha256"])
