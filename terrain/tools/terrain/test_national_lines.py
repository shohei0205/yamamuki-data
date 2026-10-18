"""全国線データの圧縮、分類、段階、索引と表示座標を確認する。"""
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
import numpy as np
from shapely.geometry import LineString
from national_lines import LEVELS,encode_lines,decode_lines,batches,build_unit,generate,prune_network,network_lines,write_progress
from national_overlay import add_national_lines,choose_level
from ridges import lon_lat


class NationalLineTests(unittest.TestCase):
    def test_codec_roundtrip_keeps_shared_endpoints(self):
        lines=[LineString([(10000000,4000000),(10000001.2,4000002.7),(10000004.1,4000008)]),LineString([(10000004.1,4000008),(9999999,4000010)])]
        body,count,vertices=encode_lines(lines);result=decode_lines(body)
        self.assertEqual((count,vertices),(2,5))
        np.testing.assert_allclose(result[0],np.rint(lines[0].coords))
        np.testing.assert_array_equal(result[0][-1],result[1][0])
        with self.assertRaises(ValueError):decode_lines(gzip.compress(b'bad'))
        with self.assertRaises(ValueError):decode_lines(gzip.compress(gzip.decompress(body)[:-1]))

    def test_degenerate_lines_removed(self):
        body,count,vertices=encode_lines([LineString([(1,.1),(1,.2)])])
        self.assertEqual((count,vertices),(0,0));self.assertEqual(decode_lines(body),[])

    def test_pruning_short_leaves_merges_long_main_route(self):
        from shapely.geometry import box
        network=network_lines([LineString([(0,0),(5000,0),(10000,0),(15000,0)]),LineString([(5000,0),(5000,500)]),LineString([(10000,0),(10000,500)])])
        pruned=prune_network(network,6000,box(-100,-100,16000,1000))
        self.assertEqual(len(pruned),1)
        self.assertEqual(pruned[0].length,15000)

    def test_range_batch_keeps_duplicate_content(self):
        rows=[[1,2,100,10],[2,2,100,10],[3,2,110,10],[4,2,120,30]]
        groups=batches(rows,20)
        self.assertEqual([(o,n) for o,n,rows in groups],[(100,20),(120,30)])
        self.assertEqual(sum(len(rows) for o,n,rows in groups),4)

    def source(self,path,latitude_y=6467):
        features=lambda code:[{'geometry':{'type':'LineString','coordinates':[[0,1000],[2048,1200],[4096,1000]]},'properties':{'vt_code':code}}]
        layers=[{'name':'RvrCL','features':features(5301)},{'name':'RdCL','features':features(2721)+features(2701)},{'name':'AdmBdry','features':features(1211)+features(1212)}]
        raw=mapbox_vector_tile.encode(layers,default_options={'extents':4096,'y_coord_down':True})
        with closing(sqlite3.connect(path)) as db:
            db.execute('CREATE TABLE tiles(x INTEGER,y INTEGER,body BLOB,PRIMARY KEY(x,y))')
            db.execute('CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT)')
            for x in [14505,14506]:db.execute('INSERT INTO tiles VALUES (?,?,?)',(x,latitude_y,gzip.compress(raw)))
            values={'complete':True,'sourceSnapshot':{'etag':'test'},'sourceTileCount':2,'sourceUrl':'test'}
            db.executemany('INSERT INTO metadata VALUES (?,?)',[(k,json.dumps(v)) for k,v in values.items()]);db.commit()

    def test_all_levels_and_geographic_query(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);source=root/'source.sqlite3';output=root/'national.sqlite3';self.source(source)
            with patch('national_lines.ProcessPoolExecutor',ThreadPoolExecutor):generate(source,output,root,1)
            with closing(sqlite3.connect(output)) as db:
                self.assertEqual([z for z, in db.execute('SELECT DISTINCT level FROM line_tiles ORDER BY level')],list(LEVELS))
                self.assertEqual(set(k for k, in db.execute('SELECT DISTINCT kind FROM line_tiles')),{'river','trail','boundary'})
                self.assertEqual(db.execute('SELECT count(*) FROM line_tiles').fetchone()[0],db.execute('SELECT count(*) FROM line_bounds').fetchone()[0])
            lon,lat=lon_lat(14506*256,6467*256+64,14)
            payload={'polygon':True,'mesh':[[-4000,-4000,0],[4000,-4000,0],[-4000,4000,0],[4000,4000,0]],'indices':[0,1,2,1,3,2],'projectionCenter':[lon,lat,0],'meshStepMeters':1000,'span':8000,'coverage':{'zoom':4,'sizeKm':2}}
            add_national_lines(payload,output)
            self.assertEqual(payload['overlays']['lineLevel'],14)
            self.assertEqual(payload['overlays']['lineLevels'],list(LEVELS))
            self.assertTrue(payload['overlays']['nationalLines'])
            for kind in ['water','trail','boundary']:self.assertGreater(len(payload['overlays']['layers'][kind]['lines']),0)
            # 粗い地形が要求範囲より広くても、線は要求した 2 km 内に制限する。
            payload['coverage'].update(latitude=lat,longitude=lon)
            add_national_lines(payload,output)
            for kind in ['water','trail','boundary']:
                points=np.asarray(payload['overlays']['layers'][kind]['lines']).reshape(-1,3)
                if len(points):self.assertLessEqual(float(np.max(np.abs(points[:,:2]))),1001)
            # 拡大率だけで変わり、地形レベルは変えない。
            payload['coverage']['sizeKm']=4000;add_national_lines(payload,output)
            self.assertEqual(payload['coverage']['zoom'],4)
            self.assertLess(payload['overlays']['lineLevel'],14)
            self.assertEqual(gzip.decompress(output.with_suffix('.sqlite3.gz').read_bytes()),output.read_bytes())

    def test_hokkaido_subprefecture_boundaries_are_excluded(self):
        with tempfile.TemporaryDirectory() as folder:
            source=Path(folder)/'source.sqlite3';self.source(source,6000)
            x,y,rows,original=build_unit((str(source),14505//64,6000//64))
            self.assertGreater(original['trail'],0);self.assertEqual(original['boundary'],0)
            self.assertFalse(any(row[1]=='boundary' for row in rows))

    def test_progress_file_lock_does_not_interrupt_generation(self):
        import time
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder)
            with patch('pathlib.Path.replace',side_effect=PermissionError('locked')),patch('national_lines.time.sleep'):
                write_progress(root,'test',1,2,time.monotonic())
            self.assertEqual(list(root.glob('*.tmp.json')),[])
            write_progress(root,'test',2,2,time.monotonic())
            self.assertEqual(json.loads((root/'progress.json').read_text(encoding='utf-8'))['done'],2)

    def test_selected_levels_only(self):
        with tempfile.TemporaryDirectory() as folder:
            source=Path(folder)/'source.sqlite3';self.source(source)
            x,y,rows,original=build_unit((str(source),14505//64,6467//64,[5,7]))
            self.assertEqual({row[0] for row in rows},{5,7})
            self.assertTrue(all(row[0] in (5,7) for row in rows))

    def test_all_level_selection(self):
        self.assertEqual(list(LEVELS),[4,5,6,7,8,9,10,11,12,13,14])
        self.assertEqual(choose_level(30,35),12)
        self.assertEqual(choose_level(60,35),11)
        self.assertEqual(choose_level(200,35),10)


if __name__=='__main__':unittest.main()
