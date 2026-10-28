"""親から全中間点を生成する形式の確認。"""
import unittest
import numpy as np
from scipy.spatial import Delaunay
from mls_generated_controls import generate,encode,decode
class GeneratedTests(unittest.TestCase):
 def parent(self):
  xy=np.random.default_rng(44).uniform(-10000,10000,(64,2));return np.column_stack((xy,100+xy[:,0]*.01+xy[:,1]*.02)).astype('<f4')
 def test_all_edges_and_parent_unchanged(self):
  parent=self.parent();saved=parent.copy();tri=Delaunay(parent[:,:2]).simplices;e=np.unique(np.sort(np.vstack((tri[:,[0,1]],tri[:,[1,2]],tri[:,[2,0]])),axis=1),axis=0)
  mid=((parent[e[:,0],:2].astype(float)+parent[e[:,1],:2])*.5).astype('<f4');np.testing.assert_array_equal(generate(parent,1),mid);np.testing.assert_array_equal(parent,saved)
 def test_residual_roundtrip(self):
  p=self.parent();xy=generate(p,1);z=100+xy[:,0]*.01+xy[:,1]*.02;decoded=decode(p,encode(p,z,1,.125));np.testing.assert_array_equal(decoded[:,:2],xy);self.assertLess(float(abs(decoded[:,2]-z).max()),.063)
 def test_wrong_parent_rejected(self):
  p=self.parent();xy=generate(p,1);b=encode(p,np.zeros(len(xy)),1,1);p[0,0]+=100
  with self.assertRaises(ValueError):decode(p,b)
if __name__=='__main__':unittest.main()
