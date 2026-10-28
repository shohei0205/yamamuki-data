"""重み付き補間の基本性質と標準版との一致を確かめる。"""
import unittest
import numpy as np
from weighted_sibson import WeightedSibson,fit,encode_weighted,decode_weighted
class TestWeighted(unittest.TestCase):
 def test_basis_and_weights(self):
  rng=np.random.default_rng(42);xy=np.vstack(([[0,0],[0,1],[1,0],[1,1]],rng.random((40,2))))*1000
  p=np.column_stack((xy,xy[:,0]*.3+xy[:,1]*.2+10));m=WeightedSibson(p);q=rng.uniform(1,999,(200,2));a=m.basis(q)
  np.testing.assert_allclose(a.sum(axis=1).A.ravel(),1,atol=1e-10)
  np.testing.assert_allclose(a@p[:,2],m.model(q),atol=1e-8)
  np.testing.assert_allclose(m(q),q[:,0]*.3+q[:,1]*.2+10,atol=1e-8)
  m.weights=np.exp(rng.uniform(-1,1,len(p)));np.testing.assert_allclose(m(p[:,:2]),p[:,2],atol=1e-8)
  m.points[:,2]=42;np.testing.assert_allclose(m(q),42,atol=1e-10)
  for weights in [np.zeros(len(p)),np.full(len(p),np.nan)]:
   with self.assertRaises(ValueError):WeightedSibson(p,weights)
 def test_storage_point_order(self):
  rng=np.random.default_rng(71);xy=np.vstack(([[0,0],[0,1],[1,0],[1,1]],rng.random((30,2))))*1000;p=np.column_stack((np.round(xy*16)/16,rng.uniform(0,100,len(xy))));w=2**(rng.integers(0,129,len(p))/32-2)
  b,wb=encode_weighted(p,w);q,qw=decode_weighted(b,wb)
  from scipy.spatial import cKDTree
  _,order=cKDTree(p[:,:2]).query(q[:,:2]);np.testing.assert_array_equal(qw,w[order]);query=rng.uniform(1,999,(200,2))
  expected=p.copy();expected[:,2]=np.round(expected[:,2]*8)/8
  np.testing.assert_allclose(WeightedSibson(q,qw)(query),WeightedSibson(expected,w)(query),atol=1e-8)
  import zlib
  with self.assertRaises(ValueError):decode_weighted(b,zlib.compress(bytes([255])*len(p)))
 def test_fit(self):
  rng=np.random.default_rng(2);xy=np.vstack(([[0,0],[0,1],[1,0],[1,1]],rng.random((40,2))))*1000;p=np.column_stack((xy,rng.uniform(0,100,len(xy))));m=WeightedSibson(p);a=m.basis(rng.uniform(1,999,(400,2)));truew=np.exp(rng.uniform(-.4,.4,len(p)));target=(a@(truew*p[:,2]))/(a@truew)
  z,w,h=fit(a,target,p[:,2],10);self.assertLess(np.mean(((a@(w*z))/(a@w)-target)**2),np.mean((a@p[:,2]-target)**2));self.assertTrue(np.all((w>=.25)&(w<=4)))
if __name__=='__main__':unittest.main()
