"""元PNG DEMの座標・画素値・欠損扱いを検証する。"""
import unittest,uuid
from pathlib import Path
import numpy as np
from raw_dem_reference import RawDem,center_for,STEP
class RawDemTests(unittest.TestCase):
 @classmethod
 def setUpClass(cls):cls.source=RawDem('build/terrain-cache',center_for('yari'))
 def test_native_pixel_centers_match_exactly(self):
  s=self.source;rng=np.random.default_rng(90);ix=rng.integers(1,s.grid.shape[1]-2,100);iy=rng.integers(1,s.grid.shape[0]-2,100);xy=np.column_stack(((s.left+ix+.5)*STEP-s.center[0],(s.top+iy+.5)*STEP-s.center[1]));np.testing.assert_allclose(s.sample(xy),s.grid[iy,ix],rtol=0,atol=.001)
 def test_missing_file_is_not_sea_zero(self):
  cache=Path('build')/('missing-dem-'+uuid.uuid4().hex)
  self.assertFalse(cache.exists())
  with self.assertRaises(FileNotFoundError):RawDem(cache,center_for('yari'))
 def test_no_dem_quantization_and_all_tiles(self):
  s=self.source;self.assertEqual(len(s.manifest),225);self.assertTrue(np.isfinite(s.grid).all());fraction=np.abs(s.grid[:256,:256]/5-np.rint(s.grid[:256,:256]/5));self.assertGreater(float(fraction.max()),.1)
if __name__=='__main__':unittest.main()
