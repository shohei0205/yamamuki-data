"""生成結果の配置・点数・復元条件を検証する。"""
import json
import unittest
from pathlib import Path
import numpy as np
from mls_quant_codec import decode_quant


class PlacementTest(unittest.TestCase):
    def test_generated_models(self):
        for region in ['fuji','yari','tsukuba']:
            old = decode_quant((Path('build/mls-six-point-v3')/region/'50-six-trained/model.mlq1').read_bytes())
            for variant,count in [('six-xy',8192),('six-more',16384)]:
                folder = Path('build/mls-xy-placement-v1')/region/('50-'+variant)
                info = json.loads((folder/'info.json').read_text())
                p = decode_quant((folder/'model.mlq1').read_bytes())
                self.assertEqual(len(p),count)
                self.assertEqual(len(np.unique(p[:,:2],axis=0)),count)
                for corner in [[-16000,-16000],[-16000,16000],[16000,-16000],[16000,16000]]:
                    self.assertTrue(np.any(np.all(p[:,:2]==corner,axis=1)))
                if count==8192:self.assertFalse(np.array_equal(old[:,:2],p[:,:2]))
                self.assertAlmostEqual(info['sigmaMeters'],88.38834764831843)
                self.assertTrue(info['sameSupportAtFitAndReconstruction'])
                self.assertFalse(info['additionalDisplaySmoothing'])
                self.assertEqual(info['samples'],11222500)
                self.assertGreaterEqual(info['neighborCountMin'],6)
                self.assertEqual(info['bytes'],(folder/'model.mlq1').stat().st_size)


if __name__=='__main__':unittest.main()
