"""水域分類、地形補間、島の保持と地形面の切り抜きを確認する。"""
import gzip
import json
import math
from pathlib import Path
import sqlite3
import tempfile
import unittest

import numpy as np
from overlay_region import Surface,add_overlays,water_class
from vector_layers import category,geographic_geometry
from ridges import RADIUS


class OverlayTests(unittest.TestCase):
    def test_feature_classification(self):
        self.assertEqual(category('RdCL',{'vt_code':2721}),'trail')
        self.assertIsNone(category('RdCL',{'vt_code':2701}))
        self.assertIsNone(category('AdmBdry',{'vt_code':1212}))
        self.assertEqual(category('AdmBdry',{'vt_code':1211}),'boundary')
        self.assertIsNone(category('RvrCL',{'vt_code':5322}))
        self.assertEqual(water_class(5200),'unknown')
        self.assertEqual(water_class(5100),'sea')

    def test_vector_coordinates_use_downward_tile_y(self):
        result=geographic_geometry({'type':'LineString','coordinates':[[0,0],[4096,4096]]},0,0,1,4096)
        self.assertAlmostEqual(result['coordinates'][0][0],-180)
        self.assertGreater(result['coordinates'][0][1],85)
        self.assertAlmostEqual(result['coordinates'][1][0],0)
        self.assertAlmostEqual(result['coordinates'][1][1],0)

    def test_heights_follow_existing_triangle_connectivity(self):
        surface=Surface([[0,0,0],[100,0,0],[0,100,0],[100,100,20]],[0,1,2,1,3,2])
        heights=surface.heights([[25,25],[75,75],[101,50]])
        np.testing.assert_allclose(heights[:2],[0,10])
        self.assertTrue(np.isnan(heights[2]))

    def fixture(self,path,lake):
        from shapely.geometry import Polygon,mapping
        import shapely
        sx=math.pi/180*RADIUS*math.cos(math.radians(35));sy=math.pi/180*RADIUS
        polygon=Polygon([(20,20),(40,20),(40,40),(20,40)],holes=[[(25,25),(30,25),(30,30),(25,30)]])
        def project(x,y,z=None):return np.asarray(x)/sx+138,np.asarray(y)/sy+35
        geometry=shapely.transform(polygon,project,interleaved=False)
        db=sqlite3.connect(path)
        db.execute('CREATE TABLE features(kind TEXT,west REAL,south REAL,east REAL,north REAL,body BLOB)')
        db.execute('CREATE TABLE metadata(key TEXT,value TEXT)')
        entries=[('water',geometry,5200)]
        if lake:
            entries.extend([('lakeedge',geometry.exterior,5231),('lakeedge',geometry.interiors[0],5231)])
        for kind,g,code in entries:
            body=gzip.compress(json.dumps({'geometry':mapping(g),'properties':{'vt_code':code}}).encode())
            db.execute('INSERT INTO features VALUES (?,?,?,?,?,?)',(kind,*g.bounds,body))
        for key,value in {'bounds':[137,34,139,36],'attribution':'試験用データ'}.items():db.execute('INSERT INTO metadata VALUES (?,?)',(key,json.dumps(value)))
        db.commit();db.close()
        return {'polygon':True,'mesh':[[0,0,0],[100,0,0],[0,100,0],[100,100,0]],'indices':[0,1,2,1,3,2],'projectionCenter':[138,35,100],'meshStepMeters':1,'span':100}

    def test_unknown_water_is_not_assumed_to_be_lake(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'vectors.sqlite3';payload=self.fixture(path,False)
            add_overlays(payload,path)
            self.assertNotIn('landIndices',payload)
            self.assertEqual(payload['overlays']['estimatedLakeLevels'],0)

    def test_lake_cutout_preserves_island_and_original_terrain(self):
        from shapely.geometry import Polygon,Point
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'vectors.sqlite3';payload=self.fixture(path,True)
            add_overlays(payload,path)
            self.assertEqual(payload['indices'],[0,1,2,1,3,2])
            self.assertGreater(payload['overlays']['estimatedLakeLevels'],0)
            faces=np.asarray(payload['overlays']['layers']['water']['triangles']).reshape(-1,3,3)
            area=sum(Polygon(f[:,:2]).area for f in faces)
            self.assertAlmostEqual(area,375,delta=3)
            self.assertFalse(any(Polygon(f[:,:2]).contains(Point(27,27)) for f in faces))
            land=np.asarray(payload['mesh'])[np.asarray(payload['landIndices']).reshape(-1,3)]
            self.assertAlmostEqual(sum(Polygon(f[:,:2]).area for f in land),10000-area,delta=.1)


if __name__=='__main__':unittest.main()
