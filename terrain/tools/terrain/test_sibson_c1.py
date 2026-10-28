import unittest
import numpy as np
from sibson_c1 import SibsonC1,gradients
class TestC1(unittest.TestCase):
 def points(self):
  rng=np.random.default_rng(94);xy=np.vstack(([[0,0],[0,1000],[1000,0],[1000,1000]],rng.uniform(0,1000,(100,2))));return xy
 def test_plane_and_interpolation(self):
  xy=self.points();z=30+.2*xy[:,0]-.1*xy[:,1];p=np.column_stack((xy,z));q=np.random.default_rng(9).uniform(1,999,(200,2))
  for limited in [False,True]:
   m=SibsonC1(p,limited=limited);np.testing.assert_allclose(m(q),30+.2*q[:,0]-.1*q[:,1],atol=1e-8);np.testing.assert_allclose(m(p[:,:2]),z,atol=1e-9)
 def test_spherical_quadric(self):
  xy=self.points();z=30+.2*xy[:,0]-.1*xy[:,1]+.001*np.sum(xy*xy,axis=1);p=np.column_stack((xy,z));gradient=[.2,-.1]+.002*xy;m=SibsonC1(p,gradient=gradient);q=np.random.default_rng(7).uniform(5,995,(100,2));expected=30+.2*q[:,0]-.1*q[:,1]+.001*np.sum(q*q,axis=1);np.testing.assert_allclose(m(q),expected,atol=1e-7)
  estimated,_=gradients(p);np.testing.assert_allclose(estimated,gradient,atol=1e-9)
 def test_node_derivative(self):
  xy=self.points();z=np.sin(xy[:,0]/300)*100+xy[:,1]*.1;m=SibsonC1(np.column_stack((xy,z)));i=20;at=xy[i:i+1];eps=.0001;g=np.array([(m(at+[eps,0])-m(at-[eps,0]))[0],(m(at+[0,eps])-m(at-[0,eps]))[0]])/(2*eps);np.testing.assert_allclose(g,m.gradient[i],atol=2e-5)
 def test_limiting_preserves_neighborhood_bounds(self):
  xy=self.points();p=np.column_stack((xy,np.sin(xy[:,0]/100)*100));m=SibsonC1(p,limited=True);self.assertTrue(np.all((m.limiting>=0)&(m.limiting<=1)))
if __name__=='__main__':unittest.main()
