"""海域の穴と描画用三角形の対応を確認する。"""
import struct
import unittest
import numpy as np
from shapely.geometry import Point, Polygon
from sea_vectors import sea_polygons, encode

class SeaVectorsTest(unittest.TestCase):
    def test_empty_and_full(self):
        self.assertEqual(sea_polygons(np.ones((16,16),bool)),[])
        areas=sea_polygons(np.zeros((16,16),bool))
        self.assertAlmostEqual(sum(p.area for p in areas),1)
        wat,mesh=encode(areas)
        self.assertEqual(wat[:4],b'WAT1')
        self.assertEqual(mesh[:4],b'SEA1')
        self.assertEqual(len(mesh),8+struct.unpack_from('<I',mesh,4)[0]*8)

    def test_island_hole_stays_out_of_triangles(self):
        valid=np.zeros((64,64),bool)
        valid[20:44,20:44]=True
        areas=sea_polygons(valid)
        self.assertFalse(any(p.contains(Point(.5,.5)) for p in areas))
        wat,mesh=encode(areas)
        self.assertEqual(struct.unpack_from('<I',wat,12)[0],2)
        vertices=np.frombuffer(mesh,dtype='<f4',offset=8).reshape(-1,3,2)
        triangles=[Polygon(t) for t in vertices]
        self.assertFalse(any(t.contains(Point(.5,.5)) for t in triangles))
        self.assertAlmostEqual(sum(t.area for t in triangles),sum(p.area for p in areas),delta=.001)

    def test_land_and_sea_both_reach_outer_boundary(self):
        valid=np.zeros((64,64),bool);valid[:,:32]=True
        areas=sea_polygons(valid)
        self.assertAlmostEqual(sum(p.area for p in areas),.5,places=5)
        self.assertFalse(any(p.contains(Point(.1,.5)) for p in areas))
        self.assertTrue(any(p.contains(Point(.9,.5)) for p in areas))

if __name__=='__main__':unittest.main()
