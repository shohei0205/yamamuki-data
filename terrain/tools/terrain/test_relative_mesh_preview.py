import unittest
import numpy as np
from relative_mesh_preview import error_limits,ridge_limits,priority_limits

class RelativeErrorTests(unittest.TestCase):
 def test_absolute_altitude_does_not_change_limits(self):
  y,x=np.mgrid[:65,:65];grid=300*np.exp(-((x-30)**2+(y-30)**2)/100)
  a=error_limits(grid,np.ones_like(grid,bool),1,radii=(2,8),smoothing_meters=0)
  b=error_limits(grid+3000,np.ones_like(grid,bool),1,radii=(2,8),smoothing_meters=0)
  np.testing.assert_allclose(a,b,atol=1e-10)
 def test_tile_cut_does_not_change_limits_with_halo(self):
  rng=np.random.default_rng(3);grid=rng.normal(size=(65,65))*100;valid=np.ones_like(grid,bool)
  full=error_limits(grid,valid,1,radii=(2,8),smoothing_meters=0)
  part=error_limits(grid[8:57,8:57],valid[8:57,8:57],1,radii=(2,8),smoothing_meters=0)
  np.testing.assert_array_equal(full[16:49,16:49],part[8:41,8:41])
 def test_flat_guard_and_exact_plane(self):
  grid=np.full((65,65),3000.);valid=np.ones_like(grid,bool)
  np.testing.assert_array_equal(error_limits(grid,valid,1,radii=(2,8),smoothing_meters=0),2)
  np.testing.assert_array_equal(error_limits(grid,valid,1,radii=(2,8),flat_guard=True,smoothing_meters=0),10)
 def test_missing_data_is_not_a_deep_valley(self):
  grid=np.full((65,65),300.);valid=np.ones_like(grid,bool);grid[:,30]=0;valid[:,30]=False
  np.testing.assert_array_equal(error_limits(grid,valid,1,radii=(2,8),smoothing_meters=0),2)

 def test_smoothing_preserves_bounds_and_reduces_steps(self):
  y,x=np.mgrid[:129,:129];grid=np.where(x<64,0.,400.);valid=np.ones_like(grid,bool)
  raw=error_limits(grid,valid,1,radii=(2,8),smoothing_meters=0)
  smooth=error_limits(grid,valid,1,radii=(2,8),smoothing_meters=3)
  self.assertGreaterEqual(smooth.min(),2);self.assertLessEqual(smooth.max(),50)
  self.assertLess(np.abs(np.diff(smooth,axis=1)).max(),np.abs(np.diff(raw,axis=1)).max())
 def test_relief_scales_error_and_caps_high_mountains(self):
  y,x=np.mgrid[:65,:65];hill=np.exp(-((x-32)**2+(y-32)**2)/100);valid=np.ones_like(hill,bool)
  low=error_limits(hill*400,valid,1,radii=(8,24),smoothing_meters=2)
  high=error_limits(hill*3000,valid,1,radii=(8,24),smoothing_meters=2)
  self.assertLess(low[32,32],20);self.assertEqual(high[32,32],50)

 def test_ridge_keeps_flanks_looser_than_relative(self):
  y,x=np.mgrid[:129,:129];grid=400*np.exp(-((x-64)**2+(y-64)**2)/200);valid=np.ones_like(grid,bool)
  relative=error_limits(grid,valid,100,radii=(1000,4000))
  ridge=ridge_limits(grid,valid,100,relative)
  self.assertTrue(np.all(ridge>=relative-.011));self.assertLess(ridge[64,64],50)
  self.assertGreater(ridge[64,95],ridge[64,64]);self.assertLessEqual(ridge.max(),50)

 def test_priority_limits_promote_summit_and_ignore_absolute_altitude(self):
  y,x=np.mgrid[:129,:129];hill=400*np.exp(-((x-64)**2+(y-64)**2)/200);valid=np.ones_like(hill,bool)
  reference=error_limits(hill,valid,100,minimum=5.)
  priority=priority_limits(hill,valid,100)
  self.assertTrue(np.all(priority>=reference/4-.011));self.assertTrue(np.all(priority<=50.011))
  self.assertLess(priority[64,64],reference[64,64]);np.testing.assert_allclose(priority,priority_limits(hill+3000,valid,100))

 def test_priority_flat_ground_does_not_compete_with_summits(self):
  y,x=np.mgrid[:129,:129];plane=x.astype(float);valid=np.ones_like(plane,bool)
  priority=priority_limits(plane,valid,100)
  self.assertGreater(priority[64,64],49.)

 def test_lower_peak_weight_relaxes_summit_without_changing_flats(self):
  y,x=np.mgrid[:129,:129];hill=400*np.exp(-((x-64)**2+(y-64)**2)/200);valid=np.ones_like(hill,bool)
  two=priority_limits(hill,valid,100,maximum_weight=2)
  four=priority_limits(hill,valid,100,maximum_weight=4)
  self.assertTrue(np.all(two>=four-.011));self.assertGreater(two[64,64],four[64,64])
  self.assertAlmostEqual(two[0,0],four[0,0],places=2)

if __name__=='__main__':unittest.main()
