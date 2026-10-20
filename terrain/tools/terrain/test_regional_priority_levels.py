"""地域全レベル比較の定義とタイル境界を確かめる。"""
import unittest
import numpy as np
from regional_priority_levels import specifications,region_tiles,boundary_points,adaptive_boundary_points

class RegionalLevelTests(unittest.TestCase):
 def test_all_level_budgets(self):
  specs=specifications();self.assertEqual(len(specs),8)
  self.assertEqual([s['maxErrorMeters'] for s in specs],[2,5,10,25,50,100,200,400])
  for l,s in enumerate(specs):
   self.assertEqual(s['widthMeters'],1000*2**l);self.assertEqual(s['baseMaxVertices'],16384);self.assertEqual(s['additionalMaxVertices'],1024)
 def test_regions_are_centered_on_mountain_tile(self):
  for l in range(8):
   keys=region_tiles(140.1069,36.2254,l)
   self.assertEqual(len(keys),9 if l<=4 else 1);self.assertEqual(len(set(keys)),len(keys))
 def test_boundary_seed_points_are_complete_without_duplicates(self):
  for n in [129,257,513,1025,2049]:
   seeds=boundary_points(n);self.assertEqual(len(seeds),4*(n-1));self.assertLess(len(seeds),16384)
   self.assertTrue(all(x in [0,n-1] or y in [0,n-1] for x,y in seeds))

 def test_flat_boundary_has_only_four_corners(self):
  y,x=np.mgrid[:129,:129];grid=x*2.+y*.5
  self.assertEqual(len(adaptive_boundary_points(grid,.1)),4)
 def test_shared_boundary_is_identical_and_within_tolerance(self):
  y,x=np.mgrid[:129,:129];grid=40*np.sin(x*.04)+20*np.cos(y*.03)
  seeds=adaptive_boundary_points(grid,2.);self.assertLess(len(seeds),4*128)
  for side in [[(i,0) for i in range(129)],[(0,j) for j in range(129)]]:
   ids=[i for i,p in enumerate(side) if p in set(seeds)];z=np.array([grid[j,i] for i,j in side]);pred=np.interp(np.arange(129),ids,z[ids]);self.assertLessEqual(np.max(np.abs(z-pred)),2.)

if __name__=='__main__':unittest.main()
