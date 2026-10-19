"""縮小前LPFの直流保持と高周波抑制を確認する。"""
import unittest
import numpy as np
from progressive_normal_maps import sinc_lpf
class LPFTest(unittest.TestCase):
 def test_constant_and_identity(self):
  base=np.full((80,80,3),.37)
  np.testing.assert_allclose(sinc_lpf(base,4),base,atol=1e-12)
  np.testing.assert_array_equal(sinc_lpf(base,1),base)
 def test_frequency(self):
  x=np.arange(256);high=np.tile(np.cos(np.pi*x),(128,1));low=np.tile(np.cos(2*np.pi*x/80),(128,1))
  self.assertLess(np.max(np.abs(sinc_lpf(high,4)[20:-20,20:-20])),.001)
  self.assertLess(np.max(np.abs(sinc_lpf(low,4)[20:-20,20:-20]-low[20:-20,20:-20])),.01)
if __name__=='__main__':unittest.main()
