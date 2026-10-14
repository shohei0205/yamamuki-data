"""外部通信なしで GSI 専用の生成・キャッシュ・地点形式を確かめる。"""

import csv
import gzip
import hashlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
from urllib.error import HTTPError

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from download import Cache
from generate import (CSV_URL, NATURAL_URL, DEM_URL, dem_elevation, generate, inside,
                      merge, name_parts, point, read_csv, read_natural, tile_position, tiles_for_bbox, write)


def mountain_csv(name="山", kana="やま", index="1", elevation="100", lat="35", lon="139"):
    stream = io.StringIO()
    writer = csv.writer(stream)
    writer.writerow(["索引番号", "山名＜山頂名＞", "山名よみ＜山頂名よみ＞", "標高値(m)", "緯度", "経度"])
    writer.writerow([index, name, kana, elevation, lat, lon])
    return stream.getvalue().encode("cp932")


def natural(name="山", kana="やま", lat=35, lon=139, gaiji="0", kind="山", alias=""):
    return json.dumps({"type": "FeatureCollection", "features": [{"type": "Feature", "geometry": {
        "type": "Point", "coordinates": [lon, lat]}, "properties": {"type": kind, "name": name,
        "kana": kana, "gaijiFlg": gaiji, "Aname": alias}}]}, ensure_ascii=False).encode()


class GenerationTests(unittest.TestCase):
    def test_multiple_natural_readings_use_first_source_reading(self):
        points, skipped = read_natural(natural(name="蔵王山", kana="ざおうざん,ざおうさん"))
        self.assertEqual(points[0]["nameReading"], "ざおうざん")
        self.assertEqual(skipped, [])

    def test_summit_alias_and_map_absent_name(self):
        self.assertEqual(("阿寒富士", "あかんふじ", []), name_parts("雌阿寒岳＜阿寒富士＞", "めあかんだけ＜あかんふじ＞"))
        self.assertEqual(("利尻山", "りしりざん", ["利尻富士"]), name_parts("利尻山（利尻富士）", "りしりざん（りしりふじ）"))
        self.assertEqual(("南硫黄島", "みなみいおうとう", []), name_parts("[南硫黄島]", "[みなみいおうとう]"))
        with self.assertRaises(ValueError):
            name_parts("山＜山頂＞", "やま")
        with self.assertRaises(ValueError):
            name_parts("山（別名）", "やま")

    def test_csv_has_gsi_ids_and_no_osm_fields(self):
        result = read_csv(mountain_csv())[0]
        self.assertEqual("gsi-mountain-list:1", result["id"])
        self.assertEqual("やま", result["nameReading"])
        self.assertEqual(100, result["elevationM"])
        self.assertNotIn("osmId", result)
        self.assertEqual(35, result["latitude"])

    def test_confirmed_csv_question_marks_are_repaired(self):
        for index, original, corrected in [('199', '薬?山', '薬萊山'), ('204-3', '?風岳', '屛風岳'),
                                           ('226', '?差岳', '朳差岳'), ('662', '?風山', '屛風山')]:
            p = read_csv(mountain_csv(name=original, index=index))[0]
            self.assertEqual(p['name'], corrected)
            self.assertEqual(p['id'], 'gsi-mountain-list:' + index)
        self.assertEqual(read_csv(mountain_csv(name='別の山', index='199'))[0]['name'], '別の山')
        self.assertEqual(read_csv(mountain_csv(name='薬?山', index='200'))[0]['name'], '薬?山')

    def test_invalid_csv_fails(self):
        for payload in (b"wrong,columns", mountain_csv(index="../1"), mountain_csv(elevation="NaN"),
                        mountain_csv(lat="91"), mountain_csv(kana="漢字")):
            with self.subTest(payload=payload):
                with self.assertRaises((ValueError, KeyError)):
                    read_csv(payload)
        payload = mountain_csv()
        payload += payload.splitlines(keepends=True)[1]
        with self.assertRaises(ValueError):
            read_csv(payload)

    def test_natural_filter_gaiji_and_stable_id(self):
        points, skipped = read_natural(natural(alias="別名"))
        self.assertEqual(["別名"], points[0]["aliases"])
        self.assertTrue(points[0]["id"].startswith("gsi-natural:"))
        self.assertEqual(points, read_natural(natural(alias="別名"))[0])
        self.assertEqual(([], []), read_natural(natural(kind="河川")))
        self.assertEqual([], read_natural(natural(gaiji="外字"))[0])
        self.assertEqual(1, len(read_natural(natural(gaiji="外字"))[1]))
        self.assertIsNone(read_natural(natural(kana=""))[0][0]["nameReading"])
        with self.assertRaises(ValueError):
            read_natural(b'{}')

    def test_bbox_and_request_limit(self):
        self.assertEqual((1, 1), tile_position(0, 0, 1))
        self.assertEqual(1, len(tiles_for_bbox([139.00001,35.00001,139.00002,35.00002])))
        for bounds in ([140, 35, 139, 36], [139, 36, 140, 35], [139, 90, 140, 91]):
            with self.assertRaises(ValueError):
                tiles_for_bbox(bounds)
        with self.assertRaises(ValueError):
            tiles_for_bbox([122, 20, 154, 46], limit=100)
        self.assertFalse(inside({"longitude":139,"latitude":35}, [140,36,141,37]))

    def test_merge_prefers_mountain_list_and_preserves_separate_same_names(self):
        csv_points = read_csv(mountain_csv())
        near = read_natural(natural(kana="さん", alias="別名"))[0]
        distant = read_natural(natural(lat=36))[0]
        result, conflicts = merge(csv_points, near + distant + distant)
        self.assertEqual(2, len(result))
        self.assertEqual(1, len(conflicts))
        primary = next(p for p in result if p["id"].startswith("gsi-mountain-list:"))
        self.assertEqual("やま", primary["nameReading"])
        self.assertEqual(100, primary["elevationM"])
        self.assertEqual(["別名"], primary["aliases"])
        self.assertEqual([], csv_points[0]["aliases"])

    def test_dem_values_missing_and_shape(self):
        payload = (",".join(["123.4"]*256) + "\n")*256
        self.assertEqual(123.4, dem_elevation(payload.encode(), 35,139))
        payload = (",".join(["e"]*256) + "\n")*256
        self.assertIsNone(dem_elevation(payload.encode(),35,139))
        with self.assertRaises(ValueError):
            dem_elevation(b'1,2',35,139)

    def test_generation_never_needs_osm_and_roundtrip_matches_schema(self):
        class FakeCache:
            used = {}
            def get(self, url, missing_ok=False):
                if url == CSV_URL:
                    return mountain_csv()
                self_test.assertTrue(url.startswith("https://cyberjapandata.gsi.go.jp/xyz/experimental_nnfpt/15/"))
                return natural(name="別の山", kana="べつのやま", lat=35.000015,lon=139.000015)
        self_test=self
        points, report = generate(FakeCache(), [139.00001,35.00001,139.00002,35.00002], natural=True)
        self.assertEqual(1, report["naturalRepresentativePointCount"])
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            manifest = write(points, report, output, "test")
            original = (output/'gsi-peaks.json.gz').read_bytes()
            write(points,report,output,"test")
            self.assertEqual(original,(output/'gsi-peaks.json.gz').read_bytes())
            self.assertEqual(points,json.loads(gzip.decompress(original)))
            self.assertEqual(hashlib.sha256(original).hexdigest(),manifest['sha256'])
            self.assertEqual("PDL-1.0", manifest['license'].split()[0])
            self.assertNotIn('latestPointTimestamp',manifest)
            self.assertNotIn('sourceTimestamp',manifest)
            schema=json.loads((Path(__file__).resolve().parents[3]/'points/schemas/data-v5.schema.json').read_text(encoding='utf-8'))
            properties=schema['$defs']['point']['properties']
            for p in points:
                self.assertFalse(set(p)-set(properties))
                self.assertTrue(set(schema['$defs']['point']['required'])<=set(p))
        with self.assertRaises(ValueError):
            generate(FakeCache(),natural=True)


class CacheTests(unittest.TestCase):
    def test_only_official_https_urls_are_allowed(self):
        with tempfile.TemporaryDirectory() as directory:
            cache=Cache(directory)
            for url in ('http://www.gsi.go.jp/a','https://example.com/a','file:///a'):
                with self.assertRaises(ValueError):cache.get(url)

    def test_cached_hash_and_provenance(self):
        class Response:
            status=200
            url=CSV_URL
            headers={"Last-Modified":"date","ETag":"tag"}
            def read(self):return b'csv'
            def __enter__(self):return self
            def __exit__(self,*args):pass
        with tempfile.TemporaryDirectory() as directory:
            cache=Cache(directory)
            with patch('download.urlopen',return_value=Response()) as fetch:
                self.assertEqual(b'csv',cache.get(CSV_URL))
                self.assertEqual(b'csv',Cache(directory).get(CSV_URL))
                self.assertEqual(1,fetch.call_count)
            self.assertEqual('date',cache.used[CSV_URL]['lastModified'])
            next(Path(directory).glob('*.data')).write_bytes(b'changed')
            with self.assertRaisesRegex(ValueError,'ハッシュ'):Cache(directory).get(CSV_URL)

    def test_missing_tile_is_cached_but_other_errors_are_not_ignored(self):
        with tempfile.TemporaryDirectory() as directory:
            url=NATURAL_URL.format(x=1,y=1)
            with patch('download.urlopen',side_effect=HTTPError(url,404,'missing',{},None)) as fetch:
                self.assertIsNone(Cache(directory).get(url,missing_ok=True))
                self.assertIsNone(Cache(directory).get(url,missing_ok=True))
                self.assertEqual(1,fetch.call_count)
            with patch('download.urlopen',side_effect=HTTPError(CSV_URL,403,'forbidden',{},None)):
                with self.assertRaises(HTTPError):Cache(directory).get(CSV_URL)
