"""小さい予算でも外周を保ち、端数を含めて予算を守る。"""
import unittest
from generate_shared_quality_budgets import allocate,budget_parent,spatial_seeds
import numpy as np
class BudgetTests(unittest.TestCase):
 def test_regular(self):self.assertEqual(allocate(48000,[81920,17920,92160],[2035,1216,2576]),[20480,4480,23040])
 def test_lowest(self):
  a=allocate(12000,[81920,17920,92160],[2035,1216,2576]);self.assertEqual(sum(a),12000);self.assertGreaterEqual(a[1],1280)
 def test_infeasible(self):
  with self.assertRaises(ValueError):allocate(100,[1,1,1],[4,4,4])
 def test_parent_excludes_storage_intersections(self):
  grid=np.zeros((65,65));p=np.array([[0,0,0],[64,64,0],[32,32,0],[30,30,0]],float)
  out,ranks,count,tol=budget_parent(p,[-1,-1,-1,.3],grid,12000)
  self.assertEqual(count,4);self.assertEqual(tol,50);self.assertFalse(any(np.array_equal(v,[32,32,0]) for v in out));self.assertTrue(any(np.array_equal(v,[30,30,0]) for v in out))
 def test_seed_reordering_keeps_every_point(self):
  p=np.array([[x,y,0] for x in range(8) for y in range(8)])
  a=spatial_seeds(p);self.assertEqual(set(map(tuple,a)),set(map(tuple,p[:,:2])));self.assertEqual(a,spatial_seeds(p))
if __name__=='__main__':unittest.main()
