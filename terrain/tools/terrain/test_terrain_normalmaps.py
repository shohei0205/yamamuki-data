"""法線の向き・欠損・縮小後の保存を確認する。"""
import unittest
import numpy as np
from terrain_normalmaps import normal_pixels


class NormalMapTests(unittest.TestCase):
    def test_flat_and_missing_areas_face_up(self):
        flat=normal_pixels(np.zeros((10,10),np.float32),1,4)
        self.assertTrue(np.all(flat==128))
        missing=normal_pixels(np.full((10,10),np.nan,np.float32),1,4)
        np.testing.assert_array_equal(missing,flat)

    def test_east_and_north_are_not_reversed(self):
        y,x=np.indices((10,10),dtype=np.float32)
        actual=normal_pixels(x+2*y,1,4)[0,0].astype(float)/127.5-1
        expected=np.array([-1,2])/np.sqrt(6)
        np.testing.assert_allclose(actual,expected,atol=1/127.5)

    def test_spacing_changes_slope_without_changing_height(self):
        y,x=np.indices((10,10),dtype=np.float32)
        grid=x.copy();original=grid.copy()
        close=normal_pixels(grid,1,4);far=normal_pixels(grid,np.full(8,10),4)
        self.assertLess(close[0,0,0],far[0,0,0])
        np.testing.assert_array_equal(grid,original)

    def test_invalid_dimensions_are_rejected(self):
        with self.assertRaises(ValueError):normal_pixels(np.zeros((10,10)),1,3)
        with self.assertRaises(ValueError):normal_pixels(np.zeros((10,10)),0,4)


if __name__=='__main__':unittest.main()
