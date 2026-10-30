"""削除後の全サンプル誤差、固定境界、許容誤差超過の拒否。"""
import unittest
import numpy as np
from prune_errorfit_tile import reduce_points
from sibson_aux_surface import AuxiliarySibsonSurface

class PruningTest(unittest.TestCase):
 def fixture(self,peak=0):
  xy=np.array([[-1,-1],[1,-1],[-1,1],[1,1],[0,0],[-.5,0],[.5,0],[0,-.5],[0,.5]],float);z=xy[:,0]*2+xy[:,1]*3;z[4]+=peak
  p=np.column_stack((xy,z));a=np.linspace(-.95,.95,21);x,y=np.meshgrid(a,a);q=np.column_stack((x.ravel(),y.ravel()));protected=np.arange(len(p))<4
  return p,q,protected
 def test_plane_reduces_to_fixed_corners(self):
  p,q,fixed=self.fixture();z=2*q[:,0]+3*q[:,1];mask,model,a=reduce_points(p,q,z,.0001,fixed)
  self.assertEqual(mask.sum(),4);self.assertTrue(mask[:4].all());self.assertLessEqual(a['max'],.0001);self.assertTrue(a['noFeasibleSingleRemoval'])
 def test_peak_cannot_be_removed_above_tolerance(self):
  p,q,fixed=self.fixture(100);z=AuxiliarySibsonSurface(p)(q);mask,model,a=reduce_points(p,q,z,5,fixed)
  self.assertTrue(mask[4]);self.assertLessEqual(abs(model(q)-z).max(),5)
 def test_invalid_initial_curve_rejected(self):
  p,q,fixed=self.fixture()
  with self.assertRaises(ValueError):reduce_points(p,q,np.full(len(q),100),1,fixed)

if __name__=='__main__':unittest.main()
