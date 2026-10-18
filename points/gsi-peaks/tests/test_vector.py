"""地図注記の分類・座標と、既存の山を消さない補完を確かめる。"""

import gzip
import hashlib
import io
import json
import sqlite3
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
import mapbox_vector_tile
from generate import point
from vector import annotations, extract, supplement, natural_readings
from download_vector import download, URL, USER_AGENT


def tile(features):
    return gzip.compress(mapbox_vector_tile.encode({"name": "Anno", "features": features},
                         default_options={"y_coord_down": True}), mtime=0)


def feature(name="宝篋山", code=312, coordinates=(3679, 1306), kind="Point"):
    return {"geometry": {"type": kind, "coordinates": coordinates},
            "properties": {"vt_text": name, "vt_code": code}}


class VectorTests(unittest.TestCase):
    def test_reading_from_non_mountain_natural_name_without_moving_point(self):
        with tempfile.TemporaryDirectory() as folder:
            database = Path(folder) / "natural.sqlite3"
            payload = json.dumps({"features": [{"geometry": {"type": "Point", "coordinates": [137.667781, 36.268643]},
                "properties": {"name": "長七ノ頭", "kana": "ちょうしちのあたま", "type": "岩・洞窟", "gaijiFlg": "0"}}]}).encode()
            with sqlite3.connect(database) as db:
                db.execute("CREATE TABLE tiles(status INTEGER,body BLOB,metadata TEXT)")
                db.execute("INSERT INTO tiles VALUES(200,?,?)", (gzip.compress(payload), json.dumps({"url": "natural-url", "sha256": hashlib.sha256(payload).hexdigest()})))
            db.close()
            p = point("gsi-vector:1", "長七ノ頭", None, 36.26892774, 137.66864776, 2312.62)
            before = p.copy()
            report = natural_readings([p], database)
            self.assertEqual(p, {**before, "nameReading": "ちょうしちのあたま"})
            self.assertEqual(report["applied"][0]["sources"][0]["naturalType"], "岩・洞窟")

    def test_download_resume_saves_original_and_offline_reuses_hash(self):
        body = b"original-pmtiles"
        class Response(io.BytesIO):
            def __init__(self, data, status, headers):
                super().__init__(data)
                self.status, self.headers = status, headers
        headers = {"Content-Length": str(len(body)), "ETag": '"version"', "Last-Modified": "date"}
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "optimal_bvmap-v1.pmtiles.part").write_bytes(body[:4])
            (root / "optimal_bvmap-source.json").write_text(json.dumps({"url": URL, "sizeBytes": len(body), "etag": '"version"', "lastModified": "date", "fetchedAt": "date"}))
            responses = [Response(b"", 200, headers), Response(body[4:], 206, {**headers, "Content-Range": f"bytes 4-{len(body)-1}/{len(body)}"})]
            with patch("download_vector.urlopen", side_effect=responses) as request:
                archive = download(root)
                self.assertEqual(request.call_args.args[0].get_header("Range"), "bytes=4-")
                self.assertEqual(request.call_args.args[0].get_header("If-match"), '"version"')
                self.assertEqual([call.args[0].get_header("User-agent") for call in request.call_args_list], [USER_AGENT, USER_AGENT])
            self.assertEqual(archive.read_bytes(), body)
            info = json.loads((root / "optimal_bvmap-source.json").read_bytes())
            self.assertEqual(info["sha256"], hashlib.sha256(body).hexdigest())
            with patch("download_vector.urlopen", side_effect=AssertionError("通信した")):
                self.assertEqual(download(root), archive)
            archive.write_bytes(b"corrupted")
            with self.assertRaisesRegex(ValueError, "一致しません"):
                download(root)

    def test_initial_download_identifies_tool_on_head_and_get(self):
        body = b"original-pmtiles"
        class Response(io.BytesIO):
            def __init__(self, data):
                super().__init__(data)
                self.status = 200
                self.headers = {"Content-Length": str(len(body)), "ETag": '"version"', "Last-Modified": "date"}
        with tempfile.TemporaryDirectory() as folder:
            with patch("download_vector.urlopen", side_effect=[Response(b""), Response(body)]) as request:
                archive = download(Path(folder))
            self.assertEqual(archive.read_bytes(), body)
            requests = [call.args[0] for call in request.call_args_list]
            self.assertEqual([r.get_method() for r in requests], ["HEAD", "GET"])
            self.assertEqual([r.get_header("User-agent") for r in requests], [USER_AGENT, USER_AGENT])
            self.assertIsNone(requests[1].get_header("Range"))

    def test_download_refuses_unknown_partial(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "optimal_bvmap-v1.pmtiles.part").write_bytes(b"unknown")
            with patch("download_vector.urlopen", side_effect=AssertionError("通信した")):
                with self.assertRaisesRegex(ValueError, "取得記録がありません"):
                    download(root)

    def test_offline_archive_resume_and_changed_input(self):
        from pmtiles.writer import Writer
        from pmtiles.tile import Compression, TileType, zxy_to_tileid
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            archive, database = root / "input.pmtiles", root / "extract.sqlite3"
            with archive.open("wb") as stream:
                writer = Writer(stream)
                writer.write_tile(zxy_to_tileid(16, 58277, 25697), tile([feature()]))
                writer.finalize({"tile_compression": Compression.GZIP, "tile_type": TileType.MVT}, {})
            expected = extract(archive, database)
            self.assertEqual(len(expected[0]), 1)
            with patch("vector.annotations", side_effect=AssertionError("取得済みのタイルを抽出し直した")):
                self.assertEqual(extract(archive, database), expected)
            archive.write_bytes(archive.read_bytes() + b"changed")
            with self.assertRaisesRegex(ValueError, "入力が異なります"):
                extract(archive, database)

    def test_hokyo_coordinates_and_missing_reading(self):
        points, skipped = annotations(tile([feature()]), 16, 58277, 25697)
        p = points[0]["point"]
        self.assertEqual(p["name"], "宝篋山")
        self.assertAlmostEqual(p["latitude"], 36.1675085857)
        self.assertAlmostEqual(p["longitude"], 140.1300559938)
        self.assertIsNone(p["nameReading"])
        self.assertIsNone(p["elevationM"])
        self.assertEqual(skipped, [])

    def test_only_individual_mountains_and_no_buffer_copies(self):
        payload = tile([feature(code=c) for c in [311, 312, 313, 810, 332, 661]] + [feature(coordinates=(-1, 100))])
        points, _ = annotations(payload, 16, 58277, 25697)
        self.assertEqual([r["properties"]["vt_code"] for r in points], [312, 313, 810])
        self.assertEqual(len({r["point"]["id"] for r in points}), 1)

    def test_line_label_is_reported_instead_of_inventing_summit(self):
        points, skipped = annotations(tile([feature(kind="LineString", coordinates=[[1, 2], [3, 4]])]), 16, 58277, 25697)
        self.assertEqual(points, [])
        self.assertEqual(len(skipped), 1)

    def test_preserve_id_reading_elevation_and_match_variant(self):
        original = point("gsi-natural:original", "槇寄山", "まきよせやま", 35, 139, 1188)
        records = [{"point": point("gsi-vector:new", "槙寄山", None, 35.0001, 139), "tile": [16, 1, 2], "properties": {"vt_code": 312}}]
        result, report = supplement([original], records)
        self.assertEqual(result, [original])
        self.assertEqual(len(report["matched"]), 1)

    def test_alias_matches_but_distant_same_name_remains_separate(self):
        original = point("gsi-mountain-list:1", "別名", "べつめい", 35, 139, 100, ["小田山"])
        records = [{"point": point("gsi-vector:near", "小田山", None, 35.0001, 139), "tile": [], "properties": {}},
                   {"point": point("gsi-vector:far", "小田山", None, 36, 139), "tile": [], "properties": {}}]
        result, report = supplement([original], records)
        self.assertEqual(len(result), 2)
        self.assertEqual(len(report["matched"]), 1)
        self.assertEqual(len(report["added"]), 1)

    def test_duplicates_merge_and_ambiguous_points_are_reported(self):
        original = [point("gsi-natural:1", "山", "やま", 35, 139), point("gsi-natural:2", "山", "やま", 35.0001, 139)]
        record = {"point": point("gsi-vector:1", "山", None, 35, 139), "tile": [], "properties": {}}
        result, report = supplement(original, [record])
        self.assertEqual(result, original)
        self.assertEqual(len(report["review"]), 1)
        result, report = supplement([], [record, record])
        self.assertEqual(len(result), 1)

    def test_possible_moved_label_is_deferred_without_duplicate_or_merge(self):
        original = point("gsi-natural:1", "山", "やま", 35, 139)
        record = {"point": point("gsi-vector:1", "山", None, 35.002, 139), "tile": [], "properties": {}}
        result, report = supplement([original], [record])
        self.assertEqual(result, [original])
        self.assertEqual(report["added"], [])
        self.assertEqual(len(report["review"]), 1)


if __name__ == "__main__":
    unittest.main()
