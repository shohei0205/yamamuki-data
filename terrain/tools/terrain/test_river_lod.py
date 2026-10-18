"""河川の合流点、元タイルの接続、末端の省略と縮尺選択を確認する。"""
from contextlib import closing
import gzip
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest

from shapely.geometry import LineString,box,mapping,shape
from river_lod import network_lines,simplify_network,choose_level,generate
from overlay_region import add_overlays


class RiverLODTests(unittest.TestCase):
    def test_tile_seam_is_merged_and_junction_is_fixed(self):
        lines=network_lines([LineString([(0,0),(20,1),(50,0)]),LineString([(50,0),(100,0)]),LineString([(50,0),(50,50)])])
        result=simplify_network(lines,10,0,box(-10,-10,110,110))
        self.assertEqual(len(result),3)
        self.assertEqual(sum((50.,0.) in (tuple(g.coords[0]),tuple(g.coords[-1])) for g in result),3)
        joined=network_lines([LineString([(0,0),(50,0)]),LineString([(50,0),(100,0)])])
        self.assertEqual(len(joined),1)
        self.assertEqual(joined[0].length,100)

    def test_short_leaf_removed_but_coverage_edge_preserved(self):
        lines=network_lines([LineString([(0,50),(50,50),(100,50)]),LineString([(50,50),(50,55)])])
        result=simplify_network(lines,2,60,box(0,0,100,100))
        self.assertEqual(len(result),2)
        self.assertTrue(all(g.length==50 for g in result))

    def test_scale_selection(self):
        self.assertEqual(choose_level(3600,35),6)
        self.assertEqual(choose_level(200,35),10)
        self.assertEqual(choose_level(30,35),12)
        self.assertEqual(choose_level(2,35),14)

    def test_generated_database_keeps_source_and_loads_on_coarse_terrain(self):
        with tempfile.TemporaryDirectory() as folder:
            source=Path(folder)/'source.sqlite3';output=Path(folder)/'lod.sqlite3'
            item={'geometry':mapping(LineString([(138,35),(138.01,35.005),(138.02,35)])),'properties':{'vt_code':5301}}
            body=gzip.compress(json.dumps(item).encode())
            with closing(sqlite3.connect(source)) as db:
                db.execute('CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT)')
                db.executemany('INSERT INTO metadata VALUES (?,?)',[(k,json.dumps(v)) for k,v in {'bounds':[138,35,138.02,35.01],'attribution':'試験'}.items()])
                db.execute('CREATE TABLE features(kind TEXT,west REAL,south REAL,east REAL,north REAL,body BLOB)')
                db.execute('INSERT INTO features VALUES (?,?,?,?,?,?)',('river',138,35,138.02,35.005,body))
                db.commit()
            report=generate(source,output)
            self.assertEqual(len(report['levels']),6)
            with closing(sqlite3.connect(output)) as db:
                self.assertEqual(db.execute('SELECT body FROM features').fetchone()[0],body)
                for level,b in db.execute('SELECT level,body FROM river_lod'):
                    g=shape(json.loads(gzip.decompress(b))['geometry'])
                    self.assertAlmostEqual(g.coords[0][0],138,places=6)
                    self.assertAlmostEqual(g.coords[-1][0],138.02,places=6)
            payload={'polygon':True,'mesh':[[-1000,-1000,0],[4000,-1000,0],[-1000,4000,0],[4000,4000,0]],'indices':[0,1,2,1,3,2],'projectionCenter':[138,35,0],'meshStepMeters':1000,'span':5000,'coverage':{'zoom':4,'sizeKm':2}}
            add_overlays(payload,output)
            self.assertEqual(payload['overlays']['riverLevel'],14)
            self.assertGreater(len(payload['overlays']['layers']['water']['lines']),0)
            self.assertEqual(gzip.decompress(output.with_suffix('.sqlite3.gz').read_bytes()),output.read_bytes())


if __name__=='__main__':unittest.main()
