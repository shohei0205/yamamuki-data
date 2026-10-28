import unittest
import numpy as np
from sibson_node_rounding import RoundedSibson
class TestRounding(unittest.TestCase):
 def points(self):
  theta=np.arange(16)*2*np.pi/16;xy=np.vstack(([[0,0]],np.column_stack((np.cos(theta),np.sin(theta)))*1000));return np.column_stack((xy,np.r_[1000,np.zeros(16)]))
 def test_point_exact_and_outside_unchanged(self):
  p=self.points();m=RoundedSibson(p,.4);np.testing.assert_allclose(m(p[:,:2]),p[:,2],atol=1e-9);q=np.array([[500.,0],[600,200],[-500,100]]);np.testing.assert_array_equal(m(q),m.base(q));d,_=m.tree.query(p[:,:2],k=2);self.assertTrue(np.all(m.radius<d[:,1]*.5))
 def test_tip_derivative_and_patch_boundary(self):
  m=RoundedSibson(self.points(),.4);eps=.001;h=m(np.array([[-eps,0],[0,0],[eps,0]]));self.assertLess(abs((h[1]-h[0])/eps-(h[2]-h[1])/eps),1e-4)
  r=m.radius[0];q=np.array([[r-eps,0],[r,0],[r+eps,0]]);h=m(q);base=m.base(q);np.testing.assert_allclose(h,base,atol=1e-7)
 def test_plane(self):
  p=self.points();p[:,2]=10+.2*p[:,0]-.1*p[:,1];m=RoundedSibson(p,.45);q=np.random.default_rng(42).uniform(-500,500,(100,2));np.testing.assert_allclose(m(q),10+.2*q[:,0]-.1*q[:,1],atol=1e-9)
 def test_fraction_validation(self):
  for fraction in [0,.5,1]:
   with self.assertRaises(ValueError):RoundedSibson(self.points(),fraction)
if __name__=='__main__':unittest.main()
