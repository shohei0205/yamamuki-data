"""ベクトル優先の出力と、必要な自然地名タイルだけの取得を確かめる。"""

import gzip
import csv
import hashlib
import io
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from build_vector import primary_points, elevations
from generate import point, NATURAL_URL, tile_position
from test_heights import png
from natural_sources import acquire, load, target_tiles


def record(p):
    return {"point": p, "properties": {"vt_code": 810}, "tile": [16, 1, 2]}


def natural_feature(name="槇寄山", kana="まきよせやま", lat=35, lon=139, kind="山", alias="", alias_kana=""):
    return {"properties": {"name": name, "kana": kana, "type": kind, "Aname": alias, "Akana": alias_kana},
            "latitude": lat, "longitude": lon, "sourceUrl": "natural-url"}


class PrimaryTests(unittest.TestCase):
    def test_vector_name_coordinates_and_id_win_over_natural(self):
        vector = point("gsi-vector:1", "槙寄山", None, 35.0001, 139)
        natural = point("gsi-natural:1", "槇寄山", "まきよせやま", 35, 139, 1188, ["別名"])
        points, report = primary_points([record(vector)], [], [natural], [natural_feature()])
        self.assertEqual(len(points), 1)
        p = points[0]
        self.assertEqual(p["id"], vector["id"])
        self.assertEqual(p["name"], vector["name"])
        self.assertEqual(p["latitude"], vector["latitude"])
        self.assertEqual(p["nameReading"], "まきよせやま")
        self.assertIsNone(p["elevationM"])
        self.assertIn("別名", p["aliases"])
        self.assertEqual(report["fallbackIds"], [])

    def test_mountain_list_summit_elevation_reading_and_index_win(self):
        vector = point("gsi-vector:1", "槙寄山", None, 35.0001, 139)
        mountain = point("gsi-mountain-list:14-1", "槇寄山", "まきよせやま", 35, 139, 1188)
        points, _ = primary_points([record(vector)], [mountain], [], [])
        self.assertEqual(points, [{**mountain, "name": "槙寄山", "aliases": ["槇寄山"]}])

    def test_label_offset_does_not_lose_official_summit_or_index(self):
        vector = point("gsi-vector:1", "筑波山", None, 35.005, 139)
        mountain = point("gsi-mountain-list:14-1", "筑波山", "つくばさん", 35, 139, 877)
        points, report = primary_points([record(vector)], [mountain], [], [])
        self.assertEqual(points, [mountain])
        self.assertEqual(report["fallbackIds"], [])

    def test_invalid_irrelevant_reading_does_not_stop_nationwide_generation(self):
        p = point("gsi-vector:1", "山", None, 35, 139)
        f = natural_feature(name="別の地名", kana="いら・びじ")
        points, _ = primary_points([record(p)], [], [], [f])
        self.assertIsNone(points[0]["nameReading"])

    def test_offset_reading_requires_unique_match_in_both_directions(self):
        p = point("gsi-vector:1", "山", None, 35, 139)
        f = natural_feature(name="山", kana="やま", lat=35.004)
        points, report = primary_points([record(p)], [], [], [f], reading_radius=1000)
        self.assertEqual(points[0]["nameReading"], "やま")
        self.assertGreater(report["naturalReadings"][0]["sources"][0]["distanceM"], 100)
        other = point("gsi-vector:2", "山", None, 35.008, 139)
        points, _ = primary_points([record(p), record(other)], [], [], [f], reading_radius=1000)
        self.assertTrue(all(p["nameReading"] is None for p in points))

    def test_non_mountain_natural_reading_and_alias_reading_are_used(self):
        p = point("gsi-vector:1", "長七ノ頭", None, 35, 139)
        f = natural_feature(name="別名", kana="べつめい", kind="岩・洞窟", alias="長七ノ頭", alias_kana="ちょうしちのあたま")
        points, _ = primary_points([record(p)], [], [], [f])
        self.assertEqual(points[0]["nameReading"], "ちょうしちのあたま")

    def test_missing_vector_mountains_are_added_but_possible_duplicates_deferred(self):
        p = point("gsi-vector:1", "山", None, 35, 139)
        sources = [point("gsi-natural:1", "別山", "べつやま", 36, 139), point("gsi-natural:2", "山", "やま", 35.002, 139)]
        points, report = primary_points([record(p)], [], sources, [])
        self.assertEqual({p["id"] for p in points}, {"gsi-vector:1", "gsi-natural:1"})
        self.assertEqual(report["fallbackIds"], ["gsi-natural:1"])
        self.assertEqual(len(report["review"]), 1)

    def test_duplicate_labels_and_ambiguous_mountain_index(self):
        p = point("gsi-vector:1", "山", None, 35, 139)
        copy = point("gsi-vector:2", "山", None, 35.00001, 139)
        points, report = primary_points([record(p), record(copy)], [], [], [])
        self.assertEqual(len(points), 1)
        self.assertEqual(len(report["duplicates"]), 1)
        other = point("gsi-vector:3", "山", None, 35.0005, 139)
        mountain = point("gsi-mountain-list:1", "山", "やま", 35.0002, 139, 100)
        points, report = primary_points([record(p), record(other)], [mountain], [], [])
        self.assertEqual({p["id"] for p in points}, {"gsi-vector:1", "gsi-vector:3"})
        self.assertEqual(len(report["review"]), 3)

    def test_missing_dem_value_does_not_invent_elevation(self):
        class Cache:
            def get(self, *args, **kwargs):
                return png(None)
        points = [point("gsi-vector:1", "山", None, 35, 139)]
        self.assertEqual(elevations(points, Cache())["pointCount"], 0)
        self.assertIsNone(points[0]["elevationM"])


class TargetedNaturalTests(unittest.TestCase):
    def test_generation_expands_only_missing_readings_and_reuses_404_offline(self):
        from build_vector import main, PMTILES_URL
        vector = point("gsi-vector:1", "山", None, 35, 139)
        f = natural_feature(name="山", kana="やま", lat=35.004)
        source_tile = tuple(int(v) for v in tile_position(f["latitude"], f["longitude"], 15))
        payload = json.dumps({"type": "FeatureCollection", "features": [{"type": "Feature", "properties": {**f["properties"], "gaijiFlg": "0"},
                    "geometry": {"type": "Point", "coordinates": [f["longitude"], f["latitude"]]}}]}).encode()
        stream = io.StringIO(); writer = csv.writer(stream)
        writer.writerow(["索引番号", "山名＜山頂名＞", "山名よみ＜山頂名よみ＞", "標高値(m)", "緯度", "経度"])
        writer.writerow(["1", "別山", "べつやま", "200", "36", "140"])
        csv_payload = stream.getvalue().encode("cp932")
        class FakeCache:
            def __init__(self, *args, **kwargs):
                self.used = {}
            def get(self, url, **kwargs):
                data = csv_payload if url.endswith('.csv') else png(100)
                self.used[url] = {"url": url, "fetchedAt": "date", "status": 200, "sha256": hashlib.sha256(data).hexdigest()}
                return data
        def fetch(number, area):
            x, y = area[:2]; found = (x, y) == source_tile
            return 0, 200 if found else 404, gzip.compress(payload) if found else None, json.dumps({"url": NATURAL_URL.format(x=x, y=y),
                "fetchedAt": "date", "status": 200 if found else 404, "sha256": hashlib.sha256(payload).hexdigest() if found else None})
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder); archive = root / 'input.pmtiles'; archive.write_bytes(b'archive')
            (root / 'optimal_bvmap-source.json').write_text(json.dumps({'url': PMTILES_URL, 'sha256': hashlib.sha256(b'archive').hexdigest(), 'sizeBytes': 7, 'fetchedAt': 'date'}))
            output = root / 'output'
            supplements = root / 'supplements.json'
            supplements.write_text(json.dumps({'schemaVersion': 1, 'points': [{'id': vector['id'], 'tags': ['日本百名山']}]}), encoding='utf-8')
            argv = ['vector.py', '--archive', str(archive), '--output', str(output), '--supplements', str(supplements)]
            with patch('sys.argv', argv), patch('build_vector.extract', return_value=([record(vector)], [], 1)), patch('build_vector.Cache', FakeCache), patch('build_vector.extract_heights', return_value=[]), patch('natural_sources.fetch', side_effect=fetch):
                main()
            first = (output / 'gsi-peaks.json').read_bytes()
            points = json.loads(first)["points"]
            self.assertTrue(all('provenance' in p for p in points))
            self.assertEqual(next(p for p in points if p['id'] == vector['id'])['tags'], ['日本百名山'])
            official = next(p for p in points if p['id'].startswith('gsi-mountain-list:'))
            self.assertEqual(official['provenance']['dataSource'], 'mountainList')
            self.assertEqual(official['provenance']['elevationSource'], 'mountainList')
            dem = next(p for p in points if p['id'].startswith('gsi-vector:'))
            self.assertEqual(dem['provenance']['dataSource'], 'vector')
            self.assertEqual(dem['provenance']['elevationSource'], 'dem1a_png')
            self.assertEqual(len(points), 2)
            self.assertEqual(next(p for p in points if p['name'] == '山')['nameReading'], 'やま')
            with patch('sys.argv', [*argv, '--offline']), patch('build_vector.extract', return_value=([record(vector)], [], 1)), patch('build_vector.Cache', FakeCache), patch('build_vector.extract_heights', return_value=[]), patch('natural_sources.fetch', side_effect=AssertionError('通信した')):
                main()
            self.assertEqual((output / 'gsi-peaks.json').read_bytes(), first)
            report = json.loads((output / 'report.json').read_bytes())
            self.assertEqual(report['naturalAcquisition']['networkRequests'], 0)
            self.assertEqual(report['naturalAcquisition']['network404Count'], 0)

    def test_targets_cover_boundary_and_are_unique(self):
        n = 2 ** 15
        lon = 29138 / n * 360 - 180
        import math
        lat = math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * 12848 / n))))
        p = point("gsi-vector:1", "山", None, lat, lon)
        tiles = target_tiles([p, p])
        self.assertEqual(tiles, {(29137, 12847), (29137, 12848), (29138, 12847), (29138, 12848)})

    def test_only_targeted_requests_and_404_cache_are_reused_offline(self):
        p = point("gsi-vector:1", "山", None, 35, 139)
        requested = []
        def missing(number, area):
            x, y = area[:2]
            requested.append((x, y))
            return 0, 404, None, json.dumps({"url": NATURAL_URL.format(x=x, y=y), "status": 404, "sha256": None, "fetchedAt": "date"})
        with tempfile.TemporaryDirectory() as folder:
            database = Path(folder) / "selected.sqlite3"
            with patch("natural_sources.fetch", side_effect=missing):
                selected, stats = acquire([p], database)
            self.assertEqual(set(requested), target_tiles([p]))
            self.assertLessEqual(len(requested), 4)
            self.assertEqual(stats["missingTiles"], len(requested))
            with patch("natural_sources.fetch", side_effect=AssertionError("通信した")):
                _, stats = acquire([p], database, offline=True)
                self.assertEqual(stats["networkRequests"], 0)
                self.assertEqual(load(database, selected), ([], [], [], []))

    def test_offline_missing_fails_and_limit_is_checked_before_fetch(self):
        with tempfile.TemporaryDirectory() as folder:
            database = Path(folder) / "selected.sqlite3"
            p = point("gsi-vector:1", "山", None, 35, 139)
            with patch("natural_sources.fetch", side_effect=AssertionError("通信した")):
                with self.assertRaises(FileNotFoundError):
                    acquire([p], database, offline=True)
                with self.assertRaisesRegex(ValueError, "上限"):
                    acquire([p], database, limit=0)

    def test_fetch_failure_is_not_recorded_as_404(self):
        with tempfile.TemporaryDirectory() as folder:
            database = Path(folder) / "selected.sqlite3"
            p = point("gsi-vector:1", "山", None, 35, 139)
            with patch("natural_sources.fetch", side_effect=RuntimeError("503")):
                with self.assertRaisesRegex(RuntimeError, "503"):
                    acquire([p], database)
            with sqlite3.connect(database) as db:
                self.assertEqual(db.execute("SELECT count(*) FROM responses").fetchone()[0], 0)
            db.close()

    def test_existing_national_data_and_original_bytes_are_reused(self):
        from fetch_all import open_database, bounds, store_tile, tile_at
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            inherited, destination = root / "old.sqlite3", root / "selected.sqlite3"
            bbox = (138.99, 34.99, 139.01, 35.01)
            db = open_database(inherited, bbox)
            area = bounds(bbox)
            total = (area[2] - area[0] + 1) * (area[3] - area[1] + 1)
            payload = json.dumps({"type": "FeatureCollection", "features": []}).encode()
            compressed = gzip.compress(payload)
            references = {}
            for number in range(total):
                x, y = tile_at(number, area)
                status = 200 if number == 0 else 404
                metadata = {"url": NATURAL_URL.format(x=x, y=y), "status": status, "fetchedAt": "date", "sha256": hashlib.sha256(payload).hexdigest() if status == 200 else None}
                store_tile(db, (number, status, compressed if status == 200 else None, json.dumps(metadata)), area, references)
            db.commit(); db.close()
            before = inherited.read_bytes()
            p = point("gsi-vector:1", "山", None, 35, 139)
            with patch("natural_sources.fetch", side_effect=AssertionError("通信した")):
                selected, stats = acquire([p], destination, inherited, offline=True)
            self.assertTrue(stats["includesCompletedScan"])
            self.assertEqual(stats["networkRequests"], 0)
            with patch("natural_sources.fetch", side_effect=AssertionError("通信した")):
                again, cached_stats = acquire([p], destination, offline=True)
            self.assertEqual(again, selected)
            self.assertTrue(cached_stats["includesCompletedScan"])
            with sqlite3.connect(destination) as db:
                self.assertEqual(db.execute("SELECT body FROM responses WHERE status=200").fetchone()[0], compressed)
            db.close()
            self.assertEqual(inherited.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
