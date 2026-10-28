"""自然近傍の再現性・範囲・局所性を確認する。"""
import unittest
import numpy as np
from sibson_surface import SibsonSurface,circumcircles

class SibsonTest(unittest.TestCase):
 def setUp(self):
  rng=np.random.default_rng(219)
  self.xy=np.vstack((rng.uniform(-1000,1000,(100,2)),[[-16000,-16000],[-16000,16000],[16000,-16000],[16000,16000]]))
  self.q=rng.uniform(-900,900,(1000,2))
 def test_blocked_circumcircles_are_identical(self):
  m=SibsonSurface(np.column_stack((self.xy,np.arange(len(self.xy)))));p=self.xy[m.tri];a=p[:,1]-p[:,0];b=p[:,2]-p[:,0];rhs=np.column_stack((np.sum(a*a,axis=1),np.sum(b*b,axis=1)))*.5;relative=np.linalg.solve(np.stack((a,b),axis=1),rhs[...,None])[...,0];expected=np.column_stack((p[:,0]+relative,np.sum(relative*relative,axis=1)))
  for block in [1,7,32768]:np.testing.assert_array_equal(circumcircles(self.xy,m.tri,block),expected)
 def test_plane_and_samples(self):
  z=2000+.2*self.xy[:,0]-.3*self.xy[:,1];m=SibsonSurface(np.column_stack((self.xy,z)))
  np.testing.assert_allclose(m(self.q),2000+.2*self.q[:,0]-.3*self.q[:,1],atol=1e-7,rtol=0)
  np.testing.assert_allclose(m(self.xy),z,atol=1e-8,rtol=0)
 def test_nonnegative_partition(self):
  # 各点を単位基底として評価し、非負かつ和1の座標であることを検証する。
  xy=np.array([[-16000,-16000],[-16000,16000],[16000,-16000],[16000,16000],[0,0],[400,500],[-500,600]],float)
  columns=[]
  for i in range(len(xy)):
   z=np.zeros(len(xy));z[i]=1
   columns.append(SibsonSurface(np.column_stack((xy,z)))(self.q[:100]))
  w=np.array(columns).T
  self.assertGreaterEqual(w.min(),-1e-12)
  np.testing.assert_allclose(w.sum(axis=1),1,atol=1e-12)
 def test_boundary(self):
  z=self.xy[:,0]*.1+self.xy[:,1]*.2
  q=np.array([[-16000,0],[16000,100],[123,16000],[321,-16000]])
  np.testing.assert_allclose(SibsonSurface(np.column_stack((self.xy,z)))(q),q[:,0]*.1+q[:,1]*.2,atol=1e-9)

if __name__=='__main__':unittest.main()
