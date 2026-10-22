"""焼き付け陰影の平坦部・解像度・グレースケールを確認する。"""
import unittest,io
import numpy as np
from PIL import Image
from generate_baked_relief import hillshade,image_for_size
class ReliefTest(unittest.TestCase):
 def test_flat_is_uniform(self):
  pixels=hillshade(np.full((65,65),300,np.float32));self.assertEqual(int(pixels.min()),int(pixels.max()));self.assertTrue(175<=pixels[0,0]<=180)
 def test_mountain_has_relief(self):
  y,x=np.mgrid[-32:33,-32:33];grid=800*np.exp(-(x*x+y*y)/180)
  pixels=image_for_size(grid.astype(np.float32),64,width=2000);self.assertEqual(pixels.shape,(64,64));self.assertGreater(np.ptp(pixels.astype(int)),30)
  out=io.BytesIO();Image.fromarray(pixels).save(out,format='PNG');self.assertEqual(Image.open(io.BytesIO(out.getvalue())).mode,'L')
 def test_world_coordinates_survive_resolution_change(self):
  y,x=np.mgrid[:65,:65];plane=(x*8+y*3).astype(np.float32)
  a=image_for_size(plane,64,width=2000);b=image_for_size(plane,32,width=2000)
  self.assertLessEqual(abs(float(a[8:-8,8:-8].mean())-float(b[4:-4,4:-4].mean())),1)
if __name__=='__main__':unittest.main()
