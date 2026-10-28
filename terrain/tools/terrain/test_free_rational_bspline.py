import unittest
import numpy as np
from scipy.interpolate import BSpline
from free_rational_bspline import beta,FreeRationalSpline,encode,decode
class TestFreeSpline(unittest.TestCase):
 def test_cubic_basis(self):
  x=np.linspace(-2,2,1001);b=BSpline.basis_element([-2,-1,0,1,2],extrapolate=False)
  np.testing.assert_allclose(beta(x),b(x),atol=1e-14)
  for edge in [-2,-1,0,1,2]:
   h=1e-4;self.assertLess(abs((beta(edge+h)-beta(edge))/h-(beta(edge)-beta(edge-h))/h),.001)
 def test_constant_positive_roundtrip(self):
  rng=np.random.default_rng(2);xy=rng.uniform(-1000,1000,(40,2));p=np.column_stack((xy,np.full(40,42.)));m=FreeRationalSpline(p,np.full((40,2),1000.),rng.uniform(0,6,40),np.exp(rng.uniform(-1,1,40)));q=rng.uniform(-1000,1000,(100,2));np.testing.assert_allclose(m(q),42,atol=1e-10);restored=decode(encode(m));np.testing.assert_allclose(restored(q),42,atol=1e-10)
  with self.assertRaises(ValueError):FreeRationalSpline(p,np.zeros((40,2)))
  with self.assertRaises(ValueError):m([[100000,100000]])
 def test_scattered_support_coverage(self):
  from free_rational_bspline import supports
  rng=np.random.default_rng(12);xy=rng.uniform(-8000,8000,(100,2));axis=np.linspace(-8000,8000,65);x,y=np.meshgrid(axis,axis);q=np.column_stack((x.ravel(),y.ravel()));scale,angle=supports(xy,lambda q:q[:,0]*.1+q[:,1]*.2,cover=q);m=FreeRationalSpline(np.column_stack((xy,np.ones(len(xy)))),scale,angle);np.testing.assert_allclose(m(q),1,atol=1e-12)
 def test_storage_order(self):
  rng=np.random.default_rng(7);xy=np.round(rng.uniform(-1000,1000,(30,2))*16)/16;p=np.column_stack((xy,np.round(rng.uniform(0,500,30)*8)/8));s=2**(rng.integers(2400,2800,(30,2))/256);angles=rng.integers(0,256,30)/256*2*np.pi;w=2**(rng.integers(0,129,30)/32-2);m=FreeRationalSpline(p,s,angles,w);other=decode(encode(m));q=rng.uniform(-500,500,(100,2));np.testing.assert_allclose(other(q),m(q),atol=1e-9)
if __name__=='__main__':unittest.main()
