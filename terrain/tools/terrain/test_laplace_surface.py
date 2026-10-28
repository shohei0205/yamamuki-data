"""Laplace辺長座標を独立のVoronoi図と照合する。"""
import unittest
import numpy as np
from scipy.spatial import Voronoi
from laplace_surface import LaplaceSurface
class LaplaceTests(unittest.TestCase):
 def points(self):
  xy=np.concatenate((np.array([[0,0],[1,0],[1,1],[0,1.]]),np.random.default_rng(8).uniform(.05,.95,(60,2))))
  return np.column_stack((xy,np.sin(xy[:,0]*8)+xy[:,1]**2))
 def test_independent_voronoi(self):
  p=self.points();model=LaplaceSurface(p);query=np.random.default_rng(9).uniform(.1,.9,(30,2));actual=model(query);expected=[]
  for q in query:
   v=Voronoi(np.vstack((p[:,:2],q)));total=value=0.
   for pair,edge in zip(v.ridge_points,v.ridge_vertices):
    if len(p) not in pair:continue
    self.assertNotIn(-1,edge);i=pair[0] if pair[1]==len(p) else pair[1];w=np.linalg.norm(v.vertices[edge[0]]-v.vertices[edge[1]])/np.linalg.norm(p[i,:2]-q);total+=w;value+=w*p[i,2]
   expected.append(value/total)
  np.testing.assert_allclose(actual,expected,atol=1e-8)
 def test_sites_and_plane(self):
  p=self.points();np.testing.assert_allclose(LaplaceSurface(p)(p[:,:2]),p[:,2],atol=1e-8)
  p[:,2]=3+p[:,0]*4-p[:,1]*2;q=np.random.default_rng(10).uniform(.1,.9,(100,2));np.testing.assert_allclose(LaplaceSurface(p)(q),3+q[:,0]*4-q[:,1]*2,atol=1e-8)
if __name__=='__main__':unittest.main()
