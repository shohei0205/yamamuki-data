"""圧縮標高の復元誤差と欠損位置を検証する。"""
import unittest
import numpy as np
from compact_dem import encode,decode


class CompactTests(unittest.TestCase):
    def test_roundtrip_and_missing_for_both_codecs(self):
        heights=np.array([[-12.3,0,123.7],[np.nan,3776.2,100],[50,51,52]],dtype=float)
        for step in [1,2,5]:
            for codec in ['gzip','xz']:
                restored=decode(encode(heights,step,codec))
                self.assertTrue(np.array_equal(np.isfinite(heights),np.isfinite(restored)))
                finite=np.isfinite(heights)
                self.assertLessEqual(np.abs(heights[finite]-restored[finite]).max(),step/2+.001)

    def test_no_missing_mask_and_all_missing(self):
        for heights in [np.full((4,5),12.5),np.full((4,5),np.nan)]:
            restored=decode(encode(heights,5,'xz'))
            self.assertEqual(restored.shape,heights.shape)
            self.assertTrue(np.array_equal(np.isnan(heights),np.isnan(restored)))

    def test_large_residuals_and_corruption(self):
        heights=np.array([[0,100000],[100000,0]],dtype=float)
        self.assertTrue(np.array_equal(heights,decode(encode(heights,1))))
        packed=encode(heights,1,'xz')
        with self.assertRaises(Exception):decode(packed[:-5])
        with self.assertRaises(ValueError):decode(b'BAD!'+packed[4:])


if __name__=='__main__':unittest.main()
