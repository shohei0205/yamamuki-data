"""全取得の範囲・再開・HTTP 応答の扱いを確かめる。"""
import gzip
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import fetch_all
from compact_cache import compact
from download import Cache


class AllTilesTests(unittest.TestCase):
    def test_national_bounds_and_numbering(self):
        area = fetch_all.bounds(fetch_all.NATIONAL_BBOX)
        total = (area[2] - area[0] + 1) * (area[3] - area[1] + 1)
        self.assertEqual(total, 8360266)
        self.assertEqual(fetch_all.tile_at(0, area), area[:2])
        self.assertEqual(fetch_all.tile_at(total - 1, area), area[2:])
        for invalid in (-1, total):
            with self.assertRaises(ValueError):
                fetch_all.tile_at(invalid, area)

    def test_resume_preserves_missing_and_rejects_different_area(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'tiles.sqlite3'
            db = fetch_all.open_database(path, fetch_all.NATIONAL_BBOX)
            db.execute("INSERT INTO tiles VALUES (1,404,NULL,'{}')")
            db.execute("INSERT INTO tiles VALUES (3,200,?, '{}')", (b'data',))
            db.commit()
            db.close()
            db = fetch_all.open_database(path, fetch_all.NATIONAL_BBOX)
            self.assertEqual(fetch_all.missing_numbers(db, 0, 5), [0, 2, 4])
            db.close()
            with self.assertRaises(ValueError):
                fetch_all.open_database(path, (130, 30, 140, 40))

    def test_compact_preserves_sources_metadata_and_resume(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            source, target = root / 'old.db', root / 'new.db'
            db = fetch_all.open_database(source, fetch_all.NATIONAL_BBOX)
            area = fetch_all.bounds(fetch_all.NATIONAL_BBOX)
            payload = gzip.compress(b'original GeoJSON bytes')
            records = []
            for number, status in ((0, 404), (1, 404), (2, 200)):
                x, y = fetch_all.tile_at(number, area)
                metadata = {'url': fetch_all.NATURAL_URL.format(x=x, y=y),
                            'status': status, 'fetchedAt': '2026-10-12T04:00:00Z',
                            'etag': None, 'extra': 'preserved'}
                records.append(metadata)
                db.execute('INSERT INTO tiles VALUES (?,?,?,?)',
                           (number, status, payload if status == 200 else None, json.dumps(metadata)))
            db.commit()
            db.close()
            compact(source, target)
            db = fetch_all.open_database(target, fetch_all.NATIONAL_BBOX)
            self.assertEqual(db.execute('SELECT count(*) FROM missing_metadata').fetchone()[0], 1)
            for number, status, body, metadata in db.execute('SELECT * FROM tiles ORDER BY number'):
                self.assertEqual(fetch_all.restore_metadata(db, number, status, metadata, area), records[number])
                if status == 200:
                    self.assertEqual(body, payload)
            self.assertEqual(fetch_all.missing_numbers(db, 0, 5), [3, 4])
            db.close()
            compact(target, root / 'again.db')
            with self.assertRaises(ValueError):
                compact(source, target)

    def test_finish_refuses_incomplete_scan(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            path = root / 'tiles.sqlite3'
            db = fetch_all.open_database(path, fetch_all.NATIONAL_BBOX)
            try:
                with self.assertRaisesRegex(ValueError, '完了していません'):
                    fetch_all.finish(db, path, (1, 2, 1, 2), root, root / 'cache', 'test', 1, 0)
            finally:
                db.close()

    def test_complete_scan_writes_json_and_completion_status(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            path = root / 'tiles.sqlite3'
            db = fetch_all.open_database(path, fetch_all.NATIONAL_BBOX)
            db.execute("INSERT INTO tiles VALUES (0,404,NULL,'{}')")
            db.commit()
            cache = Mock()
            cache.used = {}
            cache.get.return_value = b''
            mountain = {'id': 'gsi-mountain-list:1', 'type': 'peak', 'name': '山', 'nameReading': 'やま',
                        'latitude': 35, 'longitude': 139, 'elevationM': 100,
                        'aliases': [], 'wikipediaUrl': None, 'wikidataUrl': None}
            try:
                with patch.object(fetch_all, 'Cache', return_value=cache), patch.object(fetch_all, 'read_csv', return_value=[mountain]):
                    fetch_all.finish(db, path, (1, 2, 1, 2), root, root / 'cache', 'test', 1, 0)
                self.assertEqual(json.loads((root / 'gsi-peaks.json').read_bytes())["points"], [mountain])
                self.assertEqual(json.loads((root / 'progress.json').read_bytes())['phase'], 'complete')
                self.assertTrue(json.loads((root / 'report.json').read_bytes())['scan']['complete'])
            finally:
                db.close()

    def test_progress_retries_temporary_file_lock(self):
        with tempfile.TemporaryDirectory() as folder:
            with patch.object(Path, 'replace', side_effect=[PermissionError(), None]) as replace, patch.object(fetch_all.time, 'sleep') as sleep:
                fetch_all.status_file(Path(folder), phase='fetching')
                self.assertEqual(replace.call_count, 2)
                sleep.assert_called_once()

    def test_offline_cache_never_fetches_missing_source(self):
        with tempfile.TemporaryDirectory() as folder:
            with patch('download.urlopen') as request:
                with self.assertRaises(FileNotFoundError):
                    Cache(folder, offline=True).get(fetch_all.CSV_URL)
                request.assert_not_called()

    def test_offline_cache_restores_original_bytes(self):
        with tempfile.TemporaryDirectory() as folder:
            payload = b'original source bytes\r\n'
            key = hashlib.sha256(fetch_all.CSV_URL.encode()).hexdigest()
            root = Path(folder)
            (root / (key + '.data')).write_bytes(payload)
            (root / (key + '.json')).write_text(json.dumps({'url': fetch_all.CSV_URL, 'status': 200,
                'sha256': hashlib.sha256(payload).hexdigest()}), encoding='utf-8')
            with patch('download.urlopen') as request:
                self.assertEqual(Cache(folder, offline=True).get(fetch_all.CSV_URL), payload)
                request.assert_not_called()

    def response(self, status, body=b''):
        result = Mock(status=status)
        result.read.return_value = body
        result.getheader.return_value = None
        return result

    def test_success_stores_original_bytes_and_hash(self):
        body = b'{"type":"FeatureCollection","features":[]}'
        client = Mock()
        client.getresponse.return_value = self.response(200, body)
        with patch.object(fetch_all, 'connection', return_value=client):
            number, status, compressed, metadata = fetch_all.fetch(0, (1, 2, 1, 2))
        self.assertEqual((number, status), (0, 200))
        self.assertEqual(gzip.decompress(compressed), body)
        self.assertEqual(json.loads(metadata)['sha256'], hashlib.sha256(body).hexdigest())

    def test_only_404_is_missing(self):
        client = Mock()
        client.getresponse.return_value = self.response(404)
        with patch.object(fetch_all, 'connection', return_value=client):
            self.assertEqual(fetch_all.fetch(0, (1, 2, 1, 2))[1:3], (404, None))
        client.getresponse.return_value = self.response(403)
        with patch.object(fetch_all, 'connection', return_value=client):
            with self.assertRaisesRegex(ValueError, '403'):
                fetch_all.fetch(0, (1, 2, 1, 2))

    def test_transient_error_is_retried(self):
        client = Mock()
        client.getresponse.side_effect = [self.response(503), self.response(404)]
        with patch.object(fetch_all, 'connection', return_value=client), patch.object(fetch_all, 'disconnect'), patch.object(fetch_all.time, 'sleep') as sleep:
            self.assertEqual(fetch_all.fetch(0, (1, 2, 1, 2))[1], 404)
            sleep.assert_called_once()

    def test_invalid_success_body_is_not_recorded_as_missing(self):
        client = Mock()
        client.getresponse.return_value = self.response(200, b'{}')
        with patch.object(fetch_all, 'connection', return_value=client):
            with self.assertRaises(ValueError):
                fetch_all.fetch(0, (1, 2, 1, 2))


if __name__ == '__main__':
    unittest.main()
