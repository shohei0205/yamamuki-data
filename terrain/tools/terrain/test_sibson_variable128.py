"""可変サイズタイルの差分復元と高速影響領域抽出を確認する。"""
import unittest
import numpy as np
from sibson_variable128 import encode_subset,decode_subset,CircleIndex
from sibson_surface import SibsonSurface
from sibson_tile_seams import extract_points
class VariableTests(unittest.TestCase):
 def test_subset_parents(self):
  a=np.array([[0,0,10],[1,2,20],[5,5,30]],np.float32);b=np.array([[8,9,15],[12,14,25]],np.float32);p=np.array([[1,2,20.5],[8,9,14],[3,4,18]],np.float32)
  np.testing.assert_array_equal(decode_subset(encode_subset(p,[a,b]),[a,b]),p)
 def test_circle_index(self):
  rng=np.random.default_rng(14);xy=np.vstack(([[0,0],[100,0],[100,100],[0,100]],rng.uniform(0,100,(1000,2))));p=np.column_stack((xy,np.sin(xy[:,0])));m=SibsonSurface(p);index=CircleIndex(m)
  for _ in range(20):
   lo=rng.uniform(0,80,2);rect=[*lo,*(lo+20)];np.testing.assert_array_equal(index.extract(rect),extract_points(m,rect))
if __name__=='__main__':unittest.main()
