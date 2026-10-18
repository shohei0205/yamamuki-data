"""複数タイルの接続と欠損の扱いを確認する。"""
import base64
import gzip
import sqlite3
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import numpy as np
from PIL import Image
from region_server import region_payload, terrain_html
from compact_dem import encode


class RegionTests(unittest.TestCase):
    def test_adjacent_tiles_and_missing_are_preserved(self):
        with tempfile.TemporaryDirectory() as folder:
            cache=Path(folder)
            for x,y,height in [(1,1,100),(2,1,200),(2,2,300)]:
                encoded=height*100
                path=cache/'14'/str(x)/f'{y}.png'
                path.parent.mkdir(parents=True,exist_ok=True)
                Image.new('RGB',(256,256),(encoded>>16,(encoded>>8)&255,encoded&255)).save(path)
            with patch('region_server.tile_position',side_effect=[(1.99,1.99),(2.01,2.01)]),patch('urllib.request.urlopen') as network:
                payload=region_payload(cache,35,138,4)
                network.assert_not_called()
            field=payload['heightfield']
            heights=np.frombuffer(gzip.decompress(base64.b64decode(field['values'])),dtype='<f4').reshape(field['shape'])
            self.assertEqual(heights[0,0],100)
            self.assertEqual(heights[0,-1],200)
            self.assertEqual(heights[-1,-1],300)
            self.assertTrue(np.isnan(heights[-1,0]))
            self.assertEqual(payload['coverage']['savedTiles'],3)
            self.assertEqual(payload['coverage']['missingTiles'],1)
            self.assertNotIn('/*DATA*/',terrain_html(payload))
            packed=cache/'compact'
            for x,y,height in [(1,1,100),(2,1,200),(2,2,300)]:
                path=packed/'14'/str(x)/f'{y}.ymd'
                path.parent.mkdir(parents=True,exist_ok=True)
                path.write_bytes(encode(np.full((256,256),height+1.2),5,'xz'))
            with patch('region_server.tile_position',side_effect=[(1.99,1.99),(2.01,2.01)]):
                compact_payload=region_payload(cache,35,138,4,packed)
            restored=np.frombuffer(gzip.decompress(base64.b64decode(compact_payload['heightfield']['values'])),dtype='<f4').reshape(field['shape'])
            self.assertTrue(np.array_equal(np.isnan(heights),np.isnan(restored)))
            self.assertEqual(restored[0,0],100)
            self.assertEqual(compact_payload['coverage']['heightQuantizationMeters'],5)

    def test_no_saved_elevation_is_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            with self.assertRaisesRegex(ValueError,'保存済みの標高がありません'):
                region_payload(Path(folder),36,138,2)

    def test_invalid_region_is_rejected(self):
        with self.assertRaises(ValueError):
            region_payload(Path('unused'),36,138,100)

    def test_nationwide_request_cannot_force_finest_level(self):
        with tempfile.TemporaryDirectory() as folder:
            database=Path(folder)/'mesh.sqlite3'
            with sqlite3.connect(database) as db:
                db.execute('CREATE TABLE metadata(key TEXT,value TEXT)')
                db.execute('INSERT INTO metadata VALUES (?,?)',('schemaFormat','"yamamuki-terrain-mesh"'))
            db.close()
            def selected_level(database,lat,lon,size,zoom,view=None,**kwargs):
                return {'zoom':max(level for level in [4,6,8,10,12,14] if level<=zoom)}
            with patch('polygon_region.payload',side_effect=selected_level):
                payload=region_payload(Path(folder),35,138,4000,database=database,requested_zoom=14)
            self.assertEqual(payload['zoom'],4)

    def test_level13_is_accepted(self):
        with tempfile.TemporaryDirectory() as folder:
            database=Path(folder)/'mesh.sqlite3'
            with sqlite3.connect(database) as db:
                db.execute('CREATE TABLE metadata(key TEXT,value TEXT)')
                db.execute('INSERT INTO metadata VALUES (?,?)',('schemaFormat','"yamamuki-terrain-mesh"'))
            db.close()
            for zoom in (5,7,9,11,13):
                with self.subTest(zoom=zoom),patch('region_server.choose_zoom',return_value=14),patch('polygon_region.payload',side_effect=lambda *args,**kwargs:{'zoom':args[4]}) as read:
                    result=region_payload(Path(folder),35,138,6,database=database,requested_zoom=zoom)
                    self.assertEqual(result['zoom'],zoom)
                    self.assertEqual(read.call_args.args[4],zoom)

    def test_manual_level_is_not_lowered_and_disables_distance_lod(self):
        with tempfile.TemporaryDirectory() as folder:
            database=Path(folder)/'mesh.sqlite3'
            with sqlite3.connect(database) as db:
                db.execute('CREATE TABLE metadata(key TEXT,value TEXT)')
                db.execute('INSERT INTO metadata VALUES (?,?)',('schemaFormat','"yamamuki-terrain-mesh"'))
            db.close()
            with patch('region_server.choose_zoom',return_value=4),patch('polygon_region.payload',return_value={'coverage':{'zoom':11}}) as read:
                result=region_payload(Path(folder),35,138,6,database=database,requested_zoom=11,view=(80,180,1000),fixed=True)
            self.assertEqual(read.call_args.args[4],11)
            self.assertIsNone(read.call_args.args[5])
            self.assertEqual(result['coverage']['levelMode'],'manual')

    def test_manual_nationwide_fine_level_is_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            database=Path(folder)/'mesh.sqlite3'
            with sqlite3.connect(database) as db:
                db.execute('CREATE TABLE metadata(key TEXT,value TEXT)')
                db.execute('INSERT INTO metadata VALUES (?,?)',('schemaFormat','"yamamuki-terrain-mesh"'))
            db.close()
            with patch('polygon_region.payload') as read,self.assertRaisesRegex(ValueError,'拡大'):
                region_payload(Path(folder),35,138,3600,database=database,requested_zoom=14,fixed=True)
            read.assert_not_called()

    def test_unsupported_level_is_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            database=Path(folder)/'mesh.sqlite3'
            with sqlite3.connect(database) as db:
                db.execute('CREATE TABLE metadata(key TEXT,value TEXT)')
                db.execute('INSERT INTO metadata VALUES (?,?)',('schemaFormat','"yamamuki-terrain-mesh"'))
            db.close()
            with self.assertRaisesRegex(ValueError,'縮尺が不正'):
                region_payload(Path(folder),35,138,6,database=database,requested_zoom=3)


if __name__=='__main__':unittest.main()
