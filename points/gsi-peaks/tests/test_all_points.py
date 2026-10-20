"""全国注記の再開・山頂重複の除外・標高の引き継ぎを確認する。"""

from contextlib import closing
import gzip
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
import mapbox_vector_tile
import extract_annotations
import build_all
import fill_annotation_heights


def point(identifier, name='山', latitude=35, height=None, kind=None):
    return {'id': identifier, 'name': name, 'latitude': latitude, 'longitude': 139,
            'elevationM': height, 'type': kind, 'provenance': {'dataSource': 'vector', 'classification': '323'}}


class AllGsiTests(unittest.TestCase):
    def test_extraction_resume_and_single_point_selection(self):
        from pmtiles.writer import Writer
        from pmtiles.tile import Compression, TileType, zxy_to_tileid
        features = []
        for name, code, kind, coordinates in [('社', 661, 'Point', [100, 100]), ('123.4', 7201, 'Point', [200, 200]),
                                             ('', 810, 'Point', [300, 300]), ('線', 323, 'LineString', [[100, 100], [200, 200]]),
                                             ('複製', 661, 'Point', [-10, 100])]:
            features.append({'geometry': {'type': kind, 'coordinates': coordinates}, 'properties': {'vt_text': name, 'vt_code': code}})
        payload = gzip.compress(mapbox_vector_tile.encode({'name': 'Anno', 'features': features}, default_options={'y_coord_down': True}), mtime=0)
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            archive, database = root/'input.pmtiles', root/'annotations.sqlite3'
            with archive.open('wb') as stream:
                writer = Writer(stream)
                writer.write_tile(zxy_to_tileid(16, 58277, 25697), payload)
                writer.finalize({'tile_compression': Compression.GZIP, 'tile_type': TileType.MVT}, {})
            report = extract_annotations.extract(archive, database)
            self.assertEqual(report['pointCount'], 1)
            self.assertEqual(report['skipped'], {'numericOnly': 1, 'emptyName': 1, 'notSinglePoint': 1, 'tileBuffer': 1})
            with patch.object(extract_annotations, 'annotations', side_effect=AssertionError('抽出済み')):
                self.assertEqual(extract_annotations.extract(archive, database), report)
            with closing(sqlite3.connect(database)) as db:
                props = json.loads(db.execute('SELECT source FROM points').fetchone()[0])
                self.assertEqual(props['properties']['vt_code'], 661)
                db.execute("UPDATE state SET value='other' WHERE key='archive'")
                db.commit()
            with self.assertRaisesRegex(ValueError, '抽出元'):
                extract_annotations.extract(archive, database)

    def test_duplicate_chains_and_coordinate_exclusion(self):
        peaks = [point('gsi-vector:a', kind='peak'), point('gsi-mountain-list:1', name='本峰', kind='peak')]
        comparison = {'enrichment': [{'source': 'mountainList', 'vectorId': 'gsi-vector:b'}],
                      'duplicates': [{'vectorId': 'gsi-vector:d', 'representativeId': 'gsi-vector:c'},
                                     {'vectorId': 'gsi-vector:c', 'representativeId': 'gsi-vector:b'}]}
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            database = root/'annotations.sqlite3'
            with closing(sqlite3.connect(database)) as db:
                db.execute('CREATE TABLE state(key TEXT PRIMARY KEY,value TEXT)')
                db.execute("INSERT INTO state VALUES('complete','true')")
                db.execute('CREATE TABLE points(id TEXT PRIMARY KEY,body TEXT,code INTEGER,source TEXT)')
                for identifier in ('a', 'b', 'c', 'd', 'e', 'f'):
                    p = point(identifier, name='本峰' if identifier == 'e' else identifier)
                    db.execute('INSERT INTO points VALUES(?,?,?,?)', (identifier, json.dumps(p), 661, '{}'))
                db.commit()
            rows = list(build_all.annotation_points(database, peaks, comparison))
            self.assertEqual([p['id'] for p in rows], ['gsi-vector:f'])
            self.assertEqual(rows[0]['provenance'], {'dataSource': 'vector', 'classification': '661'})
            with closing(sqlite3.connect(database)) as db:
                db.execute("UPDATE state SET value='false'")
                db.commit()
            with self.assertRaisesRegex(ValueError, '完了'):
                list(build_all.annotation_points(database, peaks, comparison))

    def test_height_preservation_requires_same_name_and_coordinates(self):
        p = point('gsi-vector:1')
        saved = point(p['id'], height=100)
        saved['provenance']['elevationSource'] = 'dem1a_png'
        restored = build_all.preserve_height(p, {p['id']: saved})
        self.assertEqual(restored['elevationM'], 100)
        self.assertEqual(restored['provenance']['elevationSource'], 'dem1a_png')
        self.assertIsNone(p['elevationM'])
        for changed in ({**p, 'name': '別の地点'}, {**p, 'latitude': 34}):
            self.assertIsNone(build_all.preserve_height(changed, {p['id']: saved})['elevationM'])

    def test_dem_only_targets_missing_non_peak_related_annotations(self):
        rows = [point('gsi-vector:1'), point('gsi-vector:2', height=0), point('gsi-vector:3', kind='peak'),
                {**point('gsi-vector:4'), 'provenance': {'classification': '661'}}]
        self.assertEqual([p['id'] for p in fill_annotation_heights.selected_annotations(rows)], ['gsi-vector:1'])
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            source = root/'input.json'
            source.write_text(json.dumps(rows), encoding='utf-8')
            def dem(points, cache, workers):
                points[0]['elevationM'] = 123.45
                self.assertTrue(cache.offline)
                return {'pointCount': 1, 'bySource': {'dem1a_png': 1}, 'missingIds': [],
                        'assignments': [{'id': points[0]['id'], 'source': 'dem1a_png', 'elevationM': 123.45}]}
            with patch.object(fill_annotation_heights, 'fill_dem', side_effect=dem):
                report = fill_annotation_heights.build(source, root/'out', root/'cache', offline=True)
            result = json.loads((root/'out/gsi-all-points.json').read_bytes())['points']
            self.assertEqual(result[0]['elevationM'], 123.45)
            self.assertEqual(result[0]['provenance']['elevationSource'], 'dem1a_png')
            self.assertEqual(report['pointCount'], len(rows))
            self.assertIsNone(result[2]['elevationM'])
            self.assertEqual(json.loads(source.read_bytes()), rows)
