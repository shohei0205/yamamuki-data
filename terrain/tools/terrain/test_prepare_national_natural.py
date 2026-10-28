"""入力の固定・途中再開・欠損扱い・二重起動を確認する。"""
import json
from contextlib import closing
from pathlib import Path
import sqlite3
import tempfile
import unittest

from prepare_national_natural import ROOT, prepare, settings


class PreparationTests(unittest.TestCase):
    def setUp(self):
        (ROOT / 'build').mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=ROOT / 'build')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / 'source.sqlite3'
        self.cache = self.root / 'cache'
        self.output = self.root / 'output'
        self.config = ROOT / 'NATIONAL_NATURAL_SETTINGS.json'
        with closing(sqlite3.connect(self.source)) as db, db:
            db.execute('CREATE TABLE tiles(z INTEGER,x INTEGER,y INTEGER)')
            db.executemany('INSERT INTO tiles VALUES(14,?,?)', [(100, 200), (101, 200)])
        self.png = self.cache / '14/100/200.png'
        self.png.parent.mkdir(parents=True)
        self.png.write_bytes(b'\x89PNG\r\n\x1a\nfixture-unverified')

    def run_prepare(self, resume=False):
        return prepare(self.source, self.cache, self.output, self.config, resume)

    def test_missing_is_not_sea_and_no_false_completion(self):
        original = self.source.read_bytes()
        job = self.run_prepare()
        self.assertEqual(job['source_states'], {'png-unverified': 1, 'unavailable-input': 1})
        self.assertEqual(job['candidate_tiles']['14'], 2)
        self.assertFalse(job['candidate_counts_are_coverage'])
        self.assertFalse(job['complete'])
        self.assertFalse(job['engine_implemented'])
        self.assertEqual((self.output / 'settings.json').read_bytes(), self.config.read_bytes())
        self.assertEqual(original, self.source.read_bytes())
        self.assertEqual(self.run_prepare(True)['generation_id'], job['generation_id'])
        with self.assertRaisesRegex(ValueError, '--resume'):
            self.run_prepare()

    def test_same_length_modified_input_is_rejected(self):
        self.run_prepare()
        original = self.png.read_bytes()
        self.png.write_bytes(original[:-1] + b'X')
        with self.assertRaisesRegex(ValueError, '変化'):
            self.run_prepare(True)

    def test_added_source_does_not_mutate_fixed_inventory(self):
        self.run_prepare()
        with closing(sqlite3.connect(self.source)) as db, db:
            db.execute('INSERT INTO tiles VALUES(14,102,200)')
        with self.assertRaisesRegex(ValueError, '追加'):
            self.run_prepare(True)
        with closing(sqlite3.connect(self.output / 'inputs.sqlite3')) as db:
            self.assertEqual(db.execute('SELECT count(*) FROM inputs').fetchone()[0], 2)

    def test_concurrent_job_is_rejected(self):
        self.run_prepare()
        with closing(sqlite3.connect(self.output / 'run-lock.sqlite3')) as guard:
            guard.execute('BEGIN EXCLUSIVE')
            with self.assertRaises(sqlite3.OperationalError):
                self.run_prepare(True)

    def test_deferred_level_is_rejected(self):
        config = json.loads(self.config.read_text())
        config['levels'].append(dict(z=15, error_m=7.25))
        path = self.root / 'bad-settings.json'
        path.write_text(json.dumps(config))
        with self.assertRaisesRegex(ValueError, '7品質'):
            settings(path)

    def test_failed_inventory_can_resume_without_false_sea(self):
        self.run_prepare()
        with closing(sqlite3.connect(self.source)) as db, db:
            db.execute('DELETE FROM tiles WHERE x=101')
        with self.assertRaisesRegex(ValueError, '消えました'):
            self.run_prepare(True)
        progress = json.loads((self.output / 'progress.json').read_text())
        self.assertEqual(progress['stage'], 'failed')
        self.assertFalse(progress['complete'])


if __name__ == '__main__':
    unittest.main()
