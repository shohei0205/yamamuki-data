"""追加レベルの統合が既存タイルを保持することを確認する。"""
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from merge_mesh_levels import merge

class MergeTests(unittest.TestCase):
    def make(self,path,z):
        with sqlite3.connect(path) as db:
            db.execute('CREATE TABLE tiles(id INTEGER PRIMARY KEY,z INTEGER,x INTEGER,y INTEGER,data BLOB)')
            db.execute('CREATE UNIQUE INDEX coordinates ON tiles(z,x,y)')
            db.execute('CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT)')
            meta={'schemaFormat':'yamamuki-terrain-mesh','schemaVersion':1,'encoding':'YMP2','levels':[z],'meshErrorBounds':{str(z):35}}
            db.executemany('INSERT INTO metadata VALUES (?,?)',[(k,json.dumps(v)) for k,v in meta.items()])
            db.execute('INSERT INTO tiles(z,x,y,data) VALUES (?,?,?,?)',(z,1,2,bytes([z])))
        db.close()

    def test_merge_preserves_old_tiles_and_updates_levels(self):
        with tempfile.TemporaryDirectory() as folder:
            base,addition,output=[Path(folder)/name for name in ('base.db','extra.db','merged.db')]
            self.make(base,12);self.make(addition,13)
            report=merge(base,addition,output)
            self.assertTrue(report['gzipRoundtripVerified'])
            with sqlite3.connect(output) as db:
                self.assertEqual(db.execute('SELECT z,data FROM tiles ORDER BY z').fetchall(),[(12,b'\x0c'),(13,b'\x0d')])
                self.assertEqual(json.loads(db.execute("SELECT value FROM metadata WHERE key='levels'").fetchone()[0]),[13,12])
            db.close()
            with sqlite3.connect(base) as db:self.assertEqual(db.execute('SELECT count(*) FROM tiles').fetchone()[0],1)
            db.close()

    def test_existing_output_is_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'base.db';self.make(path,12)
            with self.assertRaises(ValueError):merge(path,path,path)

    def put_metadata(self,path,values):
        with sqlite3.connect(path) as db:
            db.executemany('INSERT OR REPLACE INTO metadata VALUES (?,?)',[(k,json.dumps(v)) for k,v in values.items()])
        db.close()

    def test_merge_preserves_coast_settings_and_previous_level_sources(self):
        with tempfile.TemporaryDirectory() as folder:
            base,addition,output=[Path(folder)/name for name in ('base.db','extra.db','merged.db')]
            self.make(base,8);self.make(addition,9)
            snapshot={'etag':'same-source'}
            self.put_metadata(base,{'coastlineSource':{'sourceSnapshot':snapshot},'coastlineToleranceMeters':{'8':40},'additionalLevelSources':{'13':{'bytes':123}}})
            self.put_metadata(addition,{'coastlineSource':{'sourceSnapshot':snapshot},'coastlineToleranceMeters':{'9':20}})
            merge(base,addition,output)
            with sqlite3.connect(output) as db:meta={k:json.loads(v) for k,v in db.execute('SELECT key,value FROM metadata')}
            db.close()
            self.assertEqual(meta['coastlineToleranceMeters'],{'8':40,'9':20})
            self.assertIn('13',meta['additionalLevelSources'])
            self.assertIn('9',meta['additionalLevelSources'])

    def test_merge_rejects_different_coast_source_snapshot(self):
        with tempfile.TemporaryDirectory() as folder:
            base,addition,output=[Path(folder)/name for name in ('base.db','extra.db','merged.db')]
            self.make(base,8);self.make(addition,9)
            self.put_metadata(base,{'coastlineSource':{'sourceSnapshot':{'etag':'old'}}})
            self.put_metadata(addition,{'coastlineSource':{'sourceSnapshot':{'etag':'new'}},'coastlineToleranceMeters':{'9':20}})
            with self.assertRaisesRegex(ValueError,'取得時点'):merge(base,addition,output)
