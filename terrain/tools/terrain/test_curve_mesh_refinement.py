import unittest
import numpy as np
from curve_mesh_refinement import curve_candidates,mesh_errors
class CurveMeshTest(unittest.TestCase):
 def test_flat_not_refined(self):
  y,x=np.mgrid[:33,:33];g=x*.2+y*.1;p=np.array([[0,0,g[0,0]],[32,0,g[0,32]],[0,32,g[32,0]],[32,32,g[32,32]]]);f=np.array([[0,1,2],[1,3,2]])
  self.assertEqual(curve_candidates(p,f,g,np.full_like(g,.2),np.full_like(g,.1),.01),[])
 def test_overshoot_prioritized(self):
  g=np.zeros((33,33));p=np.array([[0,0,0],[32,0,0],[0,32,0],[32,32,0]]);f=np.array([[0,1,2],[1,3,2]]);gx=g.copy();gy=g.copy();gx[0,32]=10
  q=curve_candidates(p,f,g,gx,gy,1,2);self.assertTrue(q);self.assertLessEqual(len(q),2);self.assertTrue(all(0<x<32 and 0<y<32 for (x,y),_ in q));self.assertEqual(q,sorted(q,key=lambda a:(-a[1],a[0])))
 def test_saved_mesh_error(self):
  g=np.zeros((5,5));g[2,2]=12;p=np.array([[0,0,0],[4,0,0],[0,4,0],[4,4,0]]);f=np.array([[0,1,2],[1,3,2]])
  maximum,ratio=mesh_errors(p,f,g,np.full_like(g,3));self.assertEqual(maximum,12);self.assertEqual(ratio,4)
if __name__=='__main__':unittest.main()
