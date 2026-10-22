"""頂点数・角度・外周・標高と面積を、角度制約付き補修で保つ。"""
import unittest
import numpy as np
from graded_budget_mesh import graded_mesh
from refine_shared_quality import angles

class GradedTests(unittest.TestCase):
 def test_budget_angle_and_boundary(self):
  y,x=np.mgrid[:65,:65];grid=x*.3+y*.2
  p=np.array([[0,0,grid[0,0]],[64,0,grid[0,64]],[64,64,grid[64,64]],[0,64,grid[64,0]]])
  out,f,report=graded_mesh(p,np.arange(4),grid,40)
  self.assertEqual(len(out),40);self.assertGreaterEqual(angles(out,f).min(),10.-1e-8)
  self.assertTrue(all(tuple(v) in set(map(tuple,out)) for v in p))
  np.testing.assert_allclose(out[:,2],grid[out[:,1].astype(int),out[:,0].astype(int)])
  q=out[f,:2];area=np.linalg.det(np.stack([q[:,1]-q[:,0],q[:,2]-q[:,0]],axis=2))/2
  self.assertTrue((area>0).all());self.assertAlmostEqual(area.sum(),4096)
 def test_deterministic(self):
  grid=np.zeros((33,33));p=np.array([[0,0,0],[32,0,0],[32,32,0],[0,32,0]],float)
  a=graded_mesh(p,np.arange(4),grid,16);b=graded_mesh(p,np.arange(4),grid,16)
  np.testing.assert_array_equal(a[0],b[0]);np.testing.assert_array_equal(a[1],b[1])

if __name__=='__main__':unittest.main()
