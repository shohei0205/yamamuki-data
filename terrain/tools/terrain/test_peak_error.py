import unittest
import numpy as np
from peak_error import summit_limits

class PeakPriorityTests(unittest.TestCase):
 def test_two_different_height_peaks_both_survive(self):
  y,x=np.mgrid[:201,:201];grid=3000*np.exp(-((x-50)**2+(y-100)**2)/100)+300*np.exp(-((x-145)**2+(y-100)**2)/64)
  allowed,peaks=summit_limits(grid,np.ones_like(grid,bool),20)
  for px in [50,145]:
   match=[p for p in peaks if abs(p['x']-px)<=2 and abs(p['y']-100)<=2]
   self.assertTrue(match);p=match[0];self.assertEqual(allowed[p['y'],p['x']],25)
  self.assertGreater(allowed[100,80],10)
  self.assertLessEqual(float(allowed.max()),25)
 def test_small_noise_does_not_become_a_protected_summit(self):
  y,x=np.mgrid[:101,:101];grid=5*np.sin(x/8)*np.cos(y/9)
  allowed,peaks=summit_limits(grid,np.ones_like(grid,bool),20)
  self.assertEqual(peaks,[]);np.testing.assert_allclose(allowed,10)
 def test_altitude_offset_does_not_change_selected_peaks(self):
  y,x=np.mgrid[:101,:101];grid=300*np.exp(-((x-50)**2+(y-50)**2)/100);valid=np.ones_like(grid,bool)
  a,pa=summit_limits(grid,valid,20);b,pb=summit_limits(grid+3000,valid,20)
  self.assertEqual([(p['x'],p['y']) for p in pa],[(p['x'],p['y']) for p in pb]);np.testing.assert_allclose(a,b,atol=.01)
 def test_twin_peaks_are_not_merged_by_distance_alone(self):
  y,x=np.mgrid[:151,:151];grid=300*np.exp(-((x-55)**2+(y-75)**2)/60)+280*np.exp(-((x-85)**2+(y-75)**2)/60)
  _,peaks=summit_limits(grid,np.ones_like(grid,bool),20)
  self.assertTrue(any(abs(p['x']-55)<3 for p in peaks));self.assertTrue(any(abs(p['x']-85)<3 for p in peaks))

if __name__=='__main__':unittest.main()
