import unittest
import numpy as np
from shapely.geometry import box,Polygon,MultiPolygon
import shapely
from landcover_curves import graph,evaluate

class CurveTests(unittest.TestCase):
 def test_shared_border_and_domain(self):
  zigzag=[(5,0),(4,2),(6,4),(4,6),(5,8),(5,10)]
  left=Polygon([(0,0),*zigzag,(0,10)])
  right=Polygon([*zigzag,(10,10),(10,0)])
  groups=[MultiPolygon([left]),MultiPolygon([right]),MultiPolygon(),MultiPolygon(),MultiPolygon()]
  g=graph(groups,10)
  for strength in [0,.5,1]:
   out=evaluate(g,strength,8)
   self.assertLess(out[0].intersection(out[1]).area,1e-10)
   self.assertLess(shapely.union_all(out).symmetric_difference(box(0,0,10,10)).area,1e-10)
   self.assertTrue(all(p.is_valid for p in out))
  self.assertGreater(np.count_nonzero(g["cut"]),0)
 def test_narrow_water_and_holes_survive(self):
  water=box(4.9,1,5.1,9)
  other=box(0,0,10,10).difference(water)
  g=graph([other,MultiPolygon(),MultiPolygon(),water,MultiPolygon()],10)
  out=evaluate(g,1,8)
  self.assertGreater(out[3].area,1.5)
  self.assertLess(out[0].intersection(out[3]).area,1e-10)
 def test_three_way_junction_is_fixed(self):
  a=box(0,0,5,10);b=box(5,0,10,5);c=box(5,5,10,10)
  # Node the left edge at the shared junction.
  a=Polygon([(0,0),(5,0),(5,5),(5,10),(0,10)])
  g=graph([a,b,c,MultiPolygon(),MultiPolygon()],10)
  i=np.flatnonzero(np.all(g["points"]==[5,5],axis=1))[0]
  self.assertEqual(g["cut"][i],0)
if __name__=='__main__':unittest.main()
