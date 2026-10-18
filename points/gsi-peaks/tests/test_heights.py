"""標高注記の一意な照合と、PNG DEM の解読・優先順位を確かめる。"""

import io
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from generate import point
from heights import png_elevation, fill_dem, apply_annotations, extract_heights, height_features


def png(value, size=(256, 256)):
    number = 2**23 if value is None else int(round(value*100)) % 2**24
    color = number >> 16, number >> 8 & 255, number & 255
    stream = io.BytesIO()
    Image.new("RGB", size, color).save(stream, format="PNG")
    return stream.getvalue()


class HeightsTests(unittest.TestCase):
    def test_png_positive_negative_zero_and_missing(self):
        for value in (171.87, -12.34, 0, None):
            self.assertEqual(png_elevation(png(value), 35, 139, 17), value)
        with self.assertRaises(ValueError):
            png_elevation(png(100, (1, 1)), 35, 139, 17)

    def test_dem_priority_404_and_missing_pixel(self):
        class Cache:
            def __init__(self): self.urls = []
            def get(self, url, **kwargs):
                self.urls.append(url)
                return None if '/dem1a_png/' in url else png(None) if '/dem5a_png/' in url else png(171.87)
        cache = Cache()
        points = [point("gsi-vector:1", "山", None, 35, 139)]
        report = fill_dem(points, cache, workers=1)
        self.assertEqual(points[0]['elevationM'], 171.87)
        self.assertEqual(report['bySource'], {'dem5b_png': 1})
        self.assertEqual(len(cache.urls), 3)

    def test_grouped_requests_preserve_existing_official_height(self):
        class Cache:
            def __init__(self): self.urls = []
            def get(self, url, **kwargs):
                self.urls.append(url); return png(171.87)
        cache = Cache()
        points = [point("gsi-vector:1", "山", None, 35, 139), point("gsi-vector:2", "別山", None, 35, 139),
                  point("gsi-mountain-list:1", "公式山", "こうしきやま", 36, 140, 200)]
        report = fill_dem(points, cache)
        self.assertEqual(len(cache.urls), 1)
        self.assertEqual(report['bySource'], {'dem1a_png': 2})
        self.assertEqual(points[2]['elevationM'], 200)

    def test_missing_all_sources_stays_null(self):
        class Cache:
            def get(self, *args, **kwargs): return None
        points = [point("gsi-vector:1", "山", None, 35, 139)]
        report = fill_dem(points, Cache())
        self.assertEqual(report['pointCount'], 0)
        self.assertEqual(report['missingIds'], ['gsi-vector:1'])

    def test_unique_nearby_annotation_and_official_height(self):
        p = point("gsi-vector:1", "山", None, 35, 139)
        official = point("gsi-mountain-list:1", "公式山", "こうしきやま", 36, 140, 200)
        measurements = [{'latitude': 35.0001, 'longitude':139, 'elevationM': 174, 'code':7201},
                        {'latitude':36, 'longitude':140, 'elevationM':199.5, 'code':7102}]
        report = apply_annotations([p, official], measurements)
        self.assertEqual(p['elevationM'], 174)
        self.assertEqual(official['elevationM'], 200)
        self.assertEqual(len(report['applied']), 1)
        self.assertEqual((p['latitude'],p['longitude']), (35,139))

    def test_shared_ambiguous_and_distant_annotations_are_not_assigned(self):
        measurements = [{'latitude':35, 'longitude':139, 'elevationM':174, 'code':7102}]
        points = [point("gsi-vector:1", "山", None, 35, 139),point("gsi-vector:2", "別山", None, 35.0001, 139)]
        self.assertEqual(apply_annotations(points, measurements)['applied'], [])
        self.assertTrue(all(p['elevationM'] is None for p in points))
        measurements.append({'latitude':35.00001, 'longitude':139, 'elevationM':175, 'code':7201})
        self.assertEqual(apply_annotations(points[:1], measurements)['applied'], [])
        distant = point("gsi-vector:3", "遠い山", None, 36, 140)
        self.assertEqual(apply_annotations([distant], measurements)['applied'], [])

    def test_extract_annotation_offline_and_resume(self):
        from pmtiles.writer import Writer
        from pmtiles.tile import Compression, TileType, zxy_to_tileid
        from test_vector import tile, feature
        payload = tile([feature(name="460.7", code=7102), feature(name="461", code=7201), feature(name="宝篋山",code=312), feature(name="",code=7102)])
        p = point("gsi-vector:1", "宝篋山", None, 36.1675085857, 140.1300559938)
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory); archive=root/'input.pmtiles'; database=root/'heights.sqlite3'
            with archive.open('wb') as stream:
                writer=Writer(stream);writer.write_tile(zxy_to_tileid(16,58277,25697),payload)
                writer.finalize({'tile_compression':Compression.GZIP,'tile_type':TileType.MVT}, {})
            records = extract_heights(archive,database,[p])
            self.assertEqual(sorted(r['elevationM'] for r in records),[460.7,461])
            with patch('heights.height_features',side_effect=AssertionError('再抽出した')):
                self.assertEqual(extract_heights(archive,database,[p]),records)
            with self.assertRaises(ValueError):
                height_features(tile([feature(name='unknown',code=7102)]),58277,25697)


if __name__ == '__main__':
    unittest.main()
