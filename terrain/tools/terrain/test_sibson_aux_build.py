"""補助情報構築高速化と幾何再利用が補間を変えないことを確認。"""
import unittest
import numpy as np
from sibson_aux_surface import AuxiliarySibsonSurface
from sibson_global_c1 import GlobalC1

class ExactGeometryTests(unittest.TestCase):
 def compare(self,p,queries=None):
  old=AuxiliarySibsonSurface(p,fast_build=False);new=AuxiliarySibsonSurface(p)
  for name in ['cell_off','ring','cells']:np.testing.assert_array_equal(getattr(old,name),getattr(new,name))
  if queries is not None:np.testing.assert_array_equal(old(queries),new(queries))
 def test_random_geometry_and_values(self):
  r=np.random.default_rng(41)
  for n in [30,300,3000]:
   xy=r.uniform(-4000,4000,(n,2));p=np.c_[xy,r.normal(size=n)];self.compare(p,r.uniform(-500,500,(500,2)))
 def test_high_degree_and_cocircular(self):
  a=np.arange(200)*2*np.pi/200;xy=np.c_[1000*np.cos(a),1000*np.sin(a)];xy=np.vstack(([0,0],xy,[[-3000,-3000],[3000,-3000],[-3000,3000],[3000,3000]]));self.compare(np.c_[xy,np.sin(xy[:,0]/1000)])
 def test_c1_reuses_identical_geometry(self):
  r=np.random.default_rng(3);xy=r.uniform(-1500,1500,(200,2));xy=np.vstack((xy,[[-3000,-3000],[3000,-3000],[-3000,3000],[3000,3000]]));p=np.c_[xy,1000+100*np.cos(xy[:,0]/500)+50*np.sin(xy[:,1]/400)];q=r.uniform(-1000,1000,(4096,2));old=GlobalC1(p,queries=q);new=GlobalC1(p,queries=q,surface=AuxiliarySibsonSurface(p));np.testing.assert_array_equal(old.gradient,new.gradient);np.testing.assert_array_equal(old(q),new(q))
if __name__=='__main__':unittest.main()
