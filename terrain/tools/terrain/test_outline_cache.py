"""事前生成した輪郭の読み込みと、古いキャッシュを使わないことを確認する。"""
from contextlib import redirect_stdout
import gzip
import io
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
import numpy as np
from outline_cache import load
from polygon_dem import encode_mesh
from polygon_region import payload
from unittest.mock import patch
from precompute_outlines import build,fingerprint

class OutlineCacheTests(unittest.TestCase):
    def test_generate_and_load_keeps_tile_edges_connected(self):
        with tempfile.TemporaryDirectory() as folder:
            source=Path(folder)/'mesh.sqlite3';output=source.with_name('terrain-outlines.sqlite3')
            vertices=np.array([[0,0,100],[256,0,100],[256,256,100],[0,256,100]],dtype=np.float32)
            body=encode_mesh(vertices,np.array([[0,1,2],[0,2,3]],dtype=np.uint32))
            with sqlite3.connect(source) as db:
                db.execute('CREATE TABLE tiles(z,x,y,data)')
                db.execute('CREATE TABLE metadata(key,value)')
                db.executemany('INSERT INTO metadata VALUES (?,?)',[(k,json.dumps(v)) for k,v in {'levels':[4],'meshErrorBounds':{'4':1200}}.items()])
                db.executemany('INSERT INTO tiles VALUES (?,?,?,?)',[(4,1,1,body),(4,2,1,body)])
            db.close();before=source.read_bytes()
            with redirect_stdout(io.StringIO()):report=build(source,output,levels=(4,))
            offsets,levels=load(source)
            self.assertEqual(levels,{4});self.assertEqual(source.read_bytes(),before)
            self.assertGreater(report['gzipBytes'],0)
            with patch('polygon_region.tile_position',side_effect=[(1.01,1.01),(2.99,1.99)]),patch('polygon_region.smooth_outline',side_effect=AssertionError('表示時の平滑化は禁止')):
                rendered=payload(source,35,138,120,4,outline=True)
            self.assertTrue(rendered['coverage']['outlinePrecomputed'])
            self.assertEqual(rendered['coverage']['outlineZoom'],4)

            for rows in offsets.values():
                self.assertTrue(np.isfinite(rows).all())
                self.assertTrue(np.all(np.linalg.norm(rows[:,1:3],axis=1)<=.600001))
                np.testing.assert_allclose(rows[:,3],0,atol=1e-5)
            def delta(tile,index):
                return next((row[1:] for row in offsets.get(tile,[]) if int(row[0])==index),np.zeros(3))
            np.testing.assert_array_equal(delta((4,1,1),1),delta((4,2,1),0))
            np.testing.assert_array_equal(delta((4,1,1),2),delta((4,2,1),3))
            with gzip.open(str(output)+'.gz','rb') as stream:self.assertEqual(stream.read(),output.read_bytes())
            with source.open('ab') as stream:stream.write(b'changed')
            self.assertEqual(load(source),({},set()))

    def test_missing_cache_returns_unmodified_geometry(self):
        with tempfile.TemporaryDirectory() as folder:
            source=Path(folder)/'mesh.sqlite3';source.write_bytes(b'x')
            self.assertEqual(load(source),({},set()))
