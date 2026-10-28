"""補助情報の有無で曲面を変えず、境界と保存復元も一致することを検証する。"""
import unittest
import numpy as np
from sibson_surface import SibsonSurface
from sibson_aux_surface import AuxiliarySibsonSurface
class AuxiliaryTests(unittest.TestCase):
 def points(self,offset=0):
  xy=np.vstack(([[0,0],[1,0],[1,1],[0,1]],np.random.default_rng(19).uniform(.001,.999,(100,2))));return np.column_stack((xy+offset,np.sin(xy[:,0]*13)*40+xy[:,1]*100))
 def test_random_boundary_and_translation(self):
  q=np.vstack((np.random.default_rng(23).uniform(.00001,.99999,(3000,2)),[[0,.2],[1,.7],[.2,0],[.6,1],[1e-7,.4],[.9999999,.2]]))
  for offset in [0,128000]:
   p=self.points(offset);a=SibsonSurface(p);b=AuxiliarySibsonSurface(p);np.testing.assert_allclose(b(q+offset),a(q+offset),atol=1e-6,rtol=1e-9);np.testing.assert_allclose(b(p[:,:2]),a(p[:,:2]),atol=1e-6);retained=np.unique(a.tri);np.testing.assert_allclose(b(p[retained,:2]),p[retained,2],atol=1e-8)
 def test_blob(self):
  p=self.points();a=AuxiliarySibsonSurface(p);b=AuxiliarySibsonSurface(p,blob=a.auxiliary_blob());q=np.random.default_rng(2).uniform(.1,.9,(100,2));np.testing.assert_array_equal(a(q),b(q));p[4,2]+=1
  with self.assertRaises(ValueError):AuxiliarySibsonSurface(p,blob=a.auxiliary_blob())
 def test_independent_voronoi_areas(self):
  from scipy.spatial import Voronoi
  from shapely.geometry import Polygon
  p=self.points();model=AuxiliarySibsonSurface(p);old=Voronoi(p[:,:2]);q=np.random.default_rng(33).uniform(.35,.65,(20,2));expected=[]
  for point in q:
   new=Voronoi(np.vstack((p[:,:2],point)));cell=Polygon(new.vertices[new.regions[new.point_region[-1]]]);weights=[]
   for v in range(len(p)):
    ids=old.regions[old.point_region[v]]
    if -1 in ids:continue
    weights.append((v,cell.intersection(Polygon(old.vertices[ids])).area))
   total=sum(w for _,w in weights);self.assertAlmostEqual(total,cell.area,places=10);expected.append(sum(p[v,2]*w for v,w in weights)/total)
  np.testing.assert_allclose(model(q),expected,atol=1e-8)
if __name__=='__main__':unittest.main()
