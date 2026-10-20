import unittest
import numpy as np
from refine_regional_quality import triangulate,angles,refine_group,quality_candidates
class QualityTests(unittest.TestCase):
 def tile(self,points):return dict(points=[list(p) for p in points],grid=np.zeros((65,65)),n=65,budget=128,added=0)
 def test_flat_square_not_refined(self):
  t=self.tile([[0,0,0],[64,0,0],[0,64,0],[64,64,0]]);refine_group({(0,0):t});self.assertEqual(t['added'],0)
 def test_shared_boundary_and_original_vertices(self):
  corners=[[0,0,0],[64,0,0],[0,64,0],[64,64,0]];a=self.tile(corners+[[63,32,0]]);b=self.tile(corners);before=(angles(a['points'],triangulate(a['points']))<10).sum();refine_group({(0,0):a,(1,0):b});after=(angles(a['points'],a['faces'])<10).sum();self.assertLess(after,before)
  self.assertEqual({p[1] for p in a['points'] if p[0]==64},{p[1] for p in b['points'] if p[0]==0});self.assertTrue(all(p in a['points'] for p in corners+[[63,32,0]]));self.assertTrue(np.isfinite(a['points']).all())
 def test_budget(self):
  t=self.tile([[0,0,0],[64,0,0],[0,64,0],[64,64,0],[1,32,0]]);t['budget']=2;refine_group({(0,0):t});self.assertLessEqual(t['added'],2)
if __name__=='__main__':unittest.main()
