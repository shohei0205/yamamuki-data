"""動的 JPEG XL の設定検証と、実際のタイル復号を確認する。"""
import struct
import unittest

import numpy as np

from dynamic_jxl import OUT, encode, validate


class DynamicJXLTest(unittest.TestCase):
    def test_invalid_settings(self):
        for options in [{'level': 0}, {'level': True}, {'normalSize': 999},
                        {'heightDistance': float('nan')}, {'normalDistance': -1}]:
            with self.assertRaises(ValueError):
                validate(options)

    def test_lossless_tiles_and_independent_normal_quality(self):
        original = encode(dict(level=11, normalSize=512, heightDistance=0, normalDistance=0))
        lossy = encode(dict(level=11, normalSize=512, heightDistance=0, normalDistance=.5))
        self.assertEqual(original['maxHeightError'], 0)
        self.assertLess(original['normalMax'], .00001)
        self.assertEqual(original['heightBytes'], lossy['heightBytes'])
        self.assertLess(lossy['normalBytes'], original['normalBytes'])
        self.assertGreater(lossy['normalP95'], 0)
        folder = OUT/original['id']
        self.assertEqual((folder/'height.yjd').stat().st_size, original['heightBytes'])
        self.assertEqual((folder/'normal.yjn').stat().st_size, original['normalBytes'])
        body = (folder/'height.yjd').read_bytes()
        self.assertEqual(body[:4], b'YJD1')
        self.assertEqual(struct.unpack_from('<HHI', body, 4), (512, 512, 4))
        self.assertEqual(np.fromfile(folder/'height.f32', dtype='<f4').size, 512*512)

    def test_modular_and_mixed_modes(self):
        with self.assertRaises(ValueError):
            encode(dict(heightMode='unknown'))
        reversible = encode(dict(level=10, normalSize=256, heightDistance=0, normalDistance=0,
                                 heightMode='modular', normalMode='modular'))
        self.assertEqual(reversible['maxHeightError'], 0)
        self.assertLess(reversible['normalMax'], .00001)
        mixed = encode(dict(level=10, normalSize=256, heightDistance=.5, normalDistance=.5,
                            heightMode='modular', normalMode='auto'))
        self.assertEqual(mixed['heightMode'], 'modular')
        self.assertEqual(mixed['normalMode'], 'auto')
        self.assertLess(mixed['heightBytes'], reversible['heightBytes'])

    def test_overlapping_lossless_tiles(self):
        result = encode(dict(level=11, normalSize=512, heightDistance=0, normalDistance=0,
                             seams='overlap'))
        self.assertEqual(result['heightPadding'], 16)
        self.assertEqual(result['normalPadding'], 16)
        self.assertEqual(result['maxHeightError'], 0)
        self.assertEqual(result['heightBoundaryErrorJump'], 0)
        self.assertLess(result['normalMax'], .00001)
        folder = OUT/result['id']
        self.assertEqual((folder/'height.yjd').read_bytes()[:4], b'YJD2')
        self.assertEqual((folder/'normal.yjn').read_bytes()[:4], b'YJN2')


if __name__ == '__main__':
    unittest.main()
