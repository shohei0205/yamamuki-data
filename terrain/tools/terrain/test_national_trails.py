"""詳細版に格納された徒歩道・橋・階段の抽出と全国データの補完を確認する。"""
from concurrent.futures import ThreadPoolExecutor
from contextlib import closing
import gzip
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch
import mapbox_vector_tile
from mapbox_vector_tile.decoder import TileData
from national_trails import strip_trails,generate,TRAIL_CODES
from national_lines import LEVELS
from vector_layers import category


class NationalTrailTests(unittest.TestCase):
    def raw(self):
        features=[{'geometry':{'type':'LineString','coordinates':[[0,1000],[2048,1100],[4096,1000]]},'properties':{'vt_code':code,'vt_flag17':1}} for code in [2701,2721,2723,2731,2734]]
        return mapbox_vector_tile.encode([{'name':'RdCL','features':features},{'name':'RvrCL','features':[features[0]]}],default_options={'extents':4096,'y_coord_down':True})

    def test_only_walking_features_including_bridges_and_steps_are_kept(self):
        result=TileData(gzip.decompress(strip_trails(self.raw())),default_options={'y_coord_down':True}).get_message()
        self.assertEqual(set(result),{'RdCL'})
        self.assertEqual({f['properties']['vt_code'] for f in result['RdCL']['features']},{2721,2723,2731,2734})
        for code in TRAIL_CODES:self.assertEqual(category('RdCL',{'vt_code':code}),'trail')
        self.assertIsNone(category('RdCL',{'vt_code':2701}))

    def test_replace_trails_preserves_other_layers_and_builds_all_levels(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);source=root/'source-trails.sqlite3';base=root/'base.sqlite3';output=root/'output.sqlite3'
            snapshot={'etag':'test'}
            with closing(sqlite3.connect(source)) as db:
                db.execute('CREATE TABLE tiles(x INTEGER,y INTEGER,body BLOB,PRIMARY KEY(x,y))');db.execute('CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT)')
                # 区画境界に達する線は広域でも省かない。
                db.execute('INSERT INTO tiles VALUES (?,?,?)',(57856,25660,strip_trails(self.raw())))
                values={'complete':True,'sourceZoom':16,'sourceTileCount':1,'savedTileCount':1,'sourceSnapshot':snapshot}
                db.executemany('INSERT INTO metadata VALUES (?,?)',[(k,json.dumps(v)) for k,v in values.items()]);db.commit()
            with closing(sqlite3.connect(base)) as db:
                db.execute('CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT)')
                db.execute('INSERT INTO metadata VALUES (?,?)',('sourceSnapshot',json.dumps(snapshot)))
                db.execute('CREATE TABLE line_tiles(id INTEGER PRIMARY KEY,level INTEGER,kind TEXT,x INTEGER,y INTEGER,body BLOB,lines INTEGER,vertices INTEGER,UNIQUE(level,kind,x,y))')
                db.execute('CREATE VIRTUAL TABLE line_bounds USING rtree(id,west,east,south,north)')
                db.execute("INSERT INTO line_tiles VALUES (1,14,'river',1,2,?,3,4)",(b'unchanged',));db.execute('INSERT INTO line_bounds VALUES (1,1,2,3,4)');db.commit()
            with patch('national_trails.ProcessPoolExecutor',ThreadPoolExecutor):generate(root,base,output,1)
            with closing(sqlite3.connect(output)) as db:
                self.assertEqual(db.execute("SELECT body FROM line_tiles WHERE kind='river'").fetchone()[0],b'unchanged')
                self.assertEqual([z for z, in db.execute("SELECT DISTINCT level FROM line_tiles WHERE kind='trail' ORDER BY level")],list(LEVELS))
                self.assertEqual(json.loads(db.execute("SELECT value FROM metadata WHERE key='trailSourceZoom'").fetchone()[0]),16)
                self.assertEqual(db.execute('PRAGMA quick_check').fetchone()[0],'ok')
            self.assertEqual(gzip.decompress(output.with_suffix('.sqlite3.gz').read_bytes()),output.read_bytes())


if __name__=='__main__':unittest.main()
