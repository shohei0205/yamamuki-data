"""局所合成の平面再現と、パッチ切替位置での連続性を検証する。"""
import unittest
import numpy as np
from partition_rbf_surface import PartitionRbfSurface

class PartitionRbfTest(unittest.TestCase):
 def test_plane(self):
  rng=np.random.default_rng(4);xy=rng.uniform(-100,100,(300,2))
  model=PartitionRbfSurface(np.column_stack((xy,3+xy[:,0]*.3-xy[:,1]*.2)))
  q=rng.uniform(-70,70,(1000,2));np.testing.assert_allclose(model(q),3+q[:,0]*.3-q[:,1]*.2,atol=1e-7)
 def test_smooth_surface(self):
  rng=np.random.default_rng(7);xy=rng.uniform(-100,100,(500,2));z=30*np.exp(-np.sum(xy**2,axis=1)/3000)
  model=PartitionRbfSurface(np.column_stack((xy,z)));q=np.column_stack((np.linspace(-65,65,5000),np.zeros(5000)))
  h=model(q);self.assertTrue(np.isfinite(h).all());self.assertLess(np.max(abs(np.diff(h,n=2))),.01)
if __name__=='__main__':unittest.main()
