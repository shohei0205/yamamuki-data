"""全国の線のレベル追加で既存データを保持することを確認する。"""
from concurrent.futures import ThreadPoolExecutor
from contextlib import closing
import gzip
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch
from shapely.geometry import LineString
from national_lines import encode_lines
from add_line_levels import add


class AdditionTests(unittest.TestCase):
    def test_append_keeps_existing_blobs_and_builds_indexes(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);base=root/'base.sqlite3';output=root/'out.sqlite3'
            with closing(sqlite3.connect(base)) as db:
                db.execute('CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT)')
                db.executemany('INSERT INTO metadata VALUES (?,?)',[(k,json.dumps(v)) for k,v in {'levels':[4],'sourceSnapshot':{'etag':'test'}}.items()])
                db.execute('CREATE TABLE line_tiles(id INTEGER PRIMARY KEY,level INTEGER,kind TEXT,x INTEGER,y INTEGER,body BLOB,lines INTEGER,vertices INTEGER,UNIQUE(level,kind,x,y))')
                db.execute('CREATE VIRTUAL TABLE line_bounds USING rtree(id,west,east,south,north)')
                db.execute("INSERT INTO line_tiles VALUES (1,4,'river',0,0,?,1,2)",(b'original',));db.execute('INSERT INTO line_bounds VALUES (1,1,2,3,4)');db.commit()
            for name in ['source.sqlite3','source-trails.sqlite3']:
                with closing(sqlite3.connect(root/name)) as db:
                    db.execute('CREATE TABLE tiles(x INTEGER,y INTEGER,body BLOB)');db.execute('INSERT INTO tiles VALUES (0,0,?)',(b'source',))
                    db.execute('CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT)')
                    db.executemany('INSERT INTO metadata VALUES (?,?)',[(k,json.dumps(v)) for k,v in {'complete':True,'sourceSnapshot':{'etag':'test'}}.items()]);db.commit()
            body,n,v=encode_lines([LineString([(15000000,4000000),(15000100,4000100)])])
            def ordinary(task):return 0,0,[(z,k,0,0,body,n,v) for z in task[3] for k in ['river','boundary']],{}
            def trails(task):return 0,0,[(z,body,n,v) for z in task[3]]
            with patch('add_line_levels.ProcessPoolExecutor',ThreadPoolExecutor),patch('add_line_levels.build_unit',ordinary),patch('add_line_levels.build_trails',trails):result=add(base,root,output,[5,7],1)
            self.assertTrue(result['existingLevelsUnchanged'])
            with closing(sqlite3.connect(output)) as db:
                self.assertEqual(db.execute('SELECT body FROM line_tiles WHERE level=4').fetchone()[0],b'original')
                self.assertEqual(db.execute('SELECT count(*) FROM line_bounds').fetchone()[0],7)
                self.assertEqual(db.execute('SELECT count(*) FROM line_tiles WHERE level IN (5,7)').fetchone()[0],6)
                self.assertEqual(json.loads(db.execute("SELECT value FROM metadata WHERE key='levels'").fetchone()[0]),[4,5,7])
            self.assertEqual(gzip.decompress(output.with_suffix('.sqlite3.gz').read_bytes()),output.read_bytes())
            with self.assertRaises(ValueError):add(output,root,root/'duplicate.sqlite3',[5,7],1)


if __name__=='__main__':unittest.main()
