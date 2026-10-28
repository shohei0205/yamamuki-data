import unittest
from pathlib import Path
import numpy as np
from compare_spline_codec import shuffle,unshuffle,gaps,ungaps,predict_integer,predict_float,zigzag,unzigzag,verify_database


class SplineCodecTests(unittest.TestCase):
    def test_byte_planes_preserve_bits(self):
        for raw in [b'',np.array([0,0x80000000,0x7fc00000,0xffffffff,1],'<u4').tobytes()]:
            self.assertEqual(unshuffle(shuffle(raw)),raw)

    def test_position_gaps(self):
        values=np.array([0,1,127,128,16384,2**32-1],'<u4')
        np.testing.assert_array_equal(ungaps(gaps(values)),values)
        self.assertEqual(len(ungaps(b'')),0)
        with self.assertRaises(ValueError):ungaps(b'\x80')

    def test_integer_refinement_preserves_plane(self):
        y,x=np.mgrid[:13,:13]
        plane=(16*x+8*y-100).astype('<i4');fine=predict_integer(plane,16)
        fy,fx=np.mgrid[:21,:21]
        np.testing.assert_array_equal(fine,8*fx+4*fy-76)
        for value in [-20000,0,30000]:
            np.testing.assert_array_equal(predict_integer(np.full((13,13),value,'<i4'),16),value)

    def test_zigzag_preserves_signed_values(self):
        value=np.array([-2**31,-32768,-1,0,1,32767,2**31-1],'<i4')
        np.testing.assert_array_equal(unzigzag(zigzag(value)),value)

    def test_float_refinement_preserves_plane(self):
        y,x=np.mgrid[:13,:13];parent=(x*.125-y*.25+100).astype('<f4')
        fy,fx=np.mgrid[:21,:21]
        np.testing.assert_array_equal(predict_float(parent,16),fx*.0625-fy*.125+99.875)

    def test_saved_parent_chains(self):
        path=Path('build/bspline-codec-comparison-v2/codecs.sqlite3')
        self.assertTrue(path.exists())
        self.assertEqual(verify_database(path,'build/bspline-surface-prototype-v3/spline.sqlite3'),231)


if __name__=='__main__':unittest.main()
