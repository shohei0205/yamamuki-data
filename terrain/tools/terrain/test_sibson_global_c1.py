import unittest
import numpy as np
from sibson_global_c1 import GlobalC1,gradient_operator
from sibson_c1 import SibsonC1
from weighted_sibson import WeightedSibson
class TestGlobalC1(unittest.TestCase):
 def points(self):
  rng=np.random.default_rng(82);xy=np.vstack(([[0,0],[0,1000],[1000,0],[1000,1000]],rng.uniform(1,999,(40,2))));return xy
 def test_operator(self):
  xy=self.points();rng=np.random.default_rng(91);p=np.column_stack((xy,rng.uniform(0,100,len(xy))));g=rng.uniform(-1,1,(len(xy),2));q=rng.uniform(1,999,(100,2));basis=WeightedSibson(p);a=basis.basis(q);m,c,z=gradient_operator(q,a,p);expected=SibsonC1(p,gradient=g,basis_model=basis).from_basis(q,a);np.testing.assert_allclose(m@g.ravel()+c,expected,atol=1e-10)
 def test_plane_and_node_values(self):
  xy=self.points();p=np.column_stack((xy,30+.2*xy[:,0]-.1*xy[:,1]));q=np.random.default_rng(17).uniform(5,995,(2000,2));m=GlobalC1(p,queries=q);np.testing.assert_allclose(m(p[:,:2]),p[:,2],atol=1e-9);np.testing.assert_allclose(m(q),30+.2*q[:,0]-.1*q[:,1],atol=.002)
 def test_native_matches_sparse(self):
  xy=self.points();p=np.column_stack((xy,np.sin(xy[:,0]/200)*100+xy[:,1]*.1));q=np.random.default_rng(29).uniform(5,995,(3000,2));m=GlobalC1(p,queries=q);expected=m.from_basis(q,m.model.basis(q));np.testing.assert_allclose(m(q),expected,atol=1e-7)
 def test_peak_and_improvement(self):
  xy=self.points();p=np.column_stack((xy,100*np.exp(-np.sum((xy-500)**2,axis=1)/80000)));q=np.random.default_rng(27).uniform(5,995,(3000,2));m=GlobalC1(p,queries=q);np.testing.assert_allclose(m(p[:,:2]),p[:,2],atol=1e-9);self.assertTrue(np.isfinite(m(q)).all());self.assertLess(m.fit_stats['fitRmse'],10);peak=int(np.argmax(p[:,2]));np.testing.assert_array_equal(m.gradient[peak],[0,0]);eps=.001;at=p[peak:peak+1,:2];h=m(np.vstack((at-[eps,0],at,at+[eps,0])));self.assertLess(abs((h[1]-h[0])/eps-(h[2]-h[1])/eps),.001)
 def test_invalid_strength(self):
  xy=self.points();p=np.column_stack((xy,np.zeros(len(xy))))
  for value in [-1,np.nan]:
   with self.assertRaises(ValueError):GlobalC1(p,strength=value)
if __name__=='__main__':unittest.main()
