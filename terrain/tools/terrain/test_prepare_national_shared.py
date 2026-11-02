from contextlib import closing
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
import hashlib
from prepare_national_shared import prepare, ROOT

class PrepareSharedTests(unittest.TestCase):
    def fixture(self,root):
        cache=root/'cache';p=cache/'14/100/200.png';p.parent.mkdir(parents=True)
        raw=b'\x89PNG\r\n\x1a\nfixture';p.write_bytes(raw)
        source=root/'coverage.sqlite3'
        with closing(sqlite3.connect(source)) as c:
            c.execute('CREATE TABLE cells(x INTEGER,y INTEGER,state TEXT,sha TEXT,bytes INTEGER)')
            c.executemany('INSERT INTO cells VALUES(?,?,?,?,?)',[(100,200,'cached',hashlib.sha256(raw).hexdigest(),len(raw)),(101,200,'verified404',None,0),(1000,1000,'verified404',None,0)])
            c.commit()
        return source,cache,p
    def test_inventory_and_resume(self):
        with tempfile.TemporaryDirectory() as td:
            r=Path(td);source,cache,_=self.fixture(r)
            a=prepare(source,cache,r/'out',ROOT/'NATIONAL_SHARED_SETTINGS.json')
            b=prepare(source,cache,r/'out',ROOT/'NATIONAL_SHARED_SETTINGS.json')
            self.assertTrue(a['inputs_verified']);self.assertFalse(a['ready_to_generate'])
            self.assertEqual(a['generation_id'],b['generation_id'])
            self.assertEqual([v['candidate_tiles'] for v in a['levels']],[1]*8)
            with closing(sqlite3.connect(r/'out/inputs.sqlite3')) as c:
                self.assertEqual(c.execute('select x,y from planned_tiles where z=13').fetchone(),(50,100))
    def test_corrupt_input_and_skip(self):
        with tempfile.TemporaryDirectory() as td:
            r=Path(td);source,cache,p=self.fixture(r);p.write_bytes(p.read_bytes()[:-1]+b'x')
            a=prepare(source,cache,r/'out',ROOT/'NATIONAL_SHARED_SETTINGS.json')
            self.assertFalse(a['inputs_verified']);self.assertEqual(len(a['issues']),1)
            b=prepare(source,cache,r/'skip',ROOT/'NATIONAL_SHARED_SETTINGS.json',False)
            self.assertFalse(b['inputs_verified']);self.assertIn('png-integrity',[v['gate'] for v in b['blockers']])
    def test_reject_old_settings_and_source_output(self):
        with tempfile.TemporaryDirectory() as td:
            r=Path(td);source,cache,_=self.fixture(r)
            old=r/'old.json';old.write_text('{"profile":"old-lpf"}')
            with self.assertRaises(ValueError):prepare(source,cache,r/'out',old)
            with self.assertRaises(ValueError):prepare(source,cache,cache/'output',ROOT/'NATIONAL_SHARED_SETTINGS.json')
if __name__=='__main__':unittest.main()
