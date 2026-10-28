"""Loop演算の重み・面接続と、実描画面による誤差評価を確認する。"""
import unittest
import numpy as np
from loop_dem_fit import loop_operator,sample_faces
class LoopFitTests(unittest.TestCase):
 def test_operator_and_plane(self):
  p=np.array([[0.,0.],[1,0],[1,1],[0,1],[.5,.5]])
  faces=np.array([[0,1,4],[1,2,4],[2,3,4],[3,0,4]])
  a,f,b=loop_operator(len(p),faces);np.testing.assert_allclose(np.asarray(a.sum(axis=1)).ravel(),1)
  self.assertEqual(len(f),16);self.assertEqual(set(b),{0,1,2,3});self.assertTrue((a.data>=0).all())
  xy=a@p;z=2+3*p[:,0]-4*p[:,1];np.testing.assert_allclose(a@z,2+3*xy[:,0]-4*xy[:,1]);self.assertLess(f.max(),len(xy))
 def test_actual_faces_not_retriangulated(self):
  xy=np.array([[0.,0.],[1,0],[1,1],[0,1]]);faces=np.array([[0,1,2],[0,2,3]]);h=np.array([0.,0,1,0]);q=np.array([[.75,.25],[.25,.75]])
  np.testing.assert_allclose(sample_faces(xy,faces,h,q),[.25,.25])
if __name__=='__main__':unittest.main()
