import unittest
import numpy as np
from scipy.spatial import Delaunay
from mesh_grading import graded_sizes

class MeshGradingTests(unittest.TestCase):
 def mesh(self):
  p=np.array([[0,0,0],[128,0,0],[128,128,0],[0,128,0]]+[[64+x,64+y,0] for x in [-2,0,2] for y in [-2,0,2]],dtype=float)
  return p,Delaunay(p[:,:2]).simplices
 def test_spacing_grows_gradually_away_from_dense_vertices(self):
  p,f=self.mesh();sizes=graded_sizes(p,f,129,10)
  self.assertLess(sizes[64,64],sizes[0,0]);self.assertGreaterEqual(sizes.min()*10,63.9)
  self.assertLessEqual(float(np.abs(np.diff(sizes,axis=0)).max()),.311)
  self.assertLessEqual(float(np.abs(np.diff(sizes,axis=1)).max()),.311)
 def test_physical_density_does_not_depend_on_source_resolution(self):
  p,f=self.mesh();a=graded_sizes(p,f,129,10);q=p.copy();q[:,:2]/=2;b=graded_sizes(q,f,65,20)
  np.testing.assert_allclose(a[::2,::2]*10,b*20,atol=.21)

if __name__=='__main__':unittest.main()
