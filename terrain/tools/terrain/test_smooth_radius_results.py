"""同一の圧縮点から新しい半径方式で正しく復元できることを確認する。"""
import hashlib
import json
import unittest
from pathlib import Path
import numpy as np
from mls_quant_codec import decode_quant
from mls_smooth_radius import MODE, model_from_metadata

ROOT = Path('build/mls-smooth-radius-v1')
VIEW = Path('build/mls-generated-preview/dem-regions')


class SmoothResultsTests(unittest.TestCase):
    def test_all_stages_keep_exact_same_shape_blob_and_report_new_evaluation(self):
        for region in ('fuji', 'yari', 'tsukuba'):
            for error in (50, 100, 200, 400, 800, 1600, 3200):
                kind = f'{error}-lpf-smooth'
                folder = ROOT/region/kind
                info = json.loads((folder/'info.json').read_text())
                old = Path('build/mls-lpf-dem-v2')/region/f'{error}-lpf-balanced'
                blob = (folder/'model.mlq1').read_bytes()
                self.assertEqual(blob, (old/'model.mlq1').read_bytes())
                self.assertEqual(info['sourceBlobSha256'], hashlib.sha256(blob).hexdigest())
                self.assertEqual(info['bytes'], len(blob))
                self.assertEqual(info['supportMode'], MODE)
                self.assertFalse(info['radiusFieldStored'])
                self.assertEqual(info['samples'], 11222500)
                self.assertEqual(info['missing'], 0)
                model = model_from_metadata(decode_quant(blob), info)
                n = info['n']; x = np.array([0, 120, 500, n]); y = np.array([n, 700, 50, 0])
                for span in (8000, 32000):
                    field = np.memmap(VIEW/region/f'{kind}-{span}.bin', dtype='<f4', mode='r', shape=(n+1, n+1, 3))
                    q = np.column_stack(((x/n-.5)*span, (y/n-.5)*span))
                    np.testing.assert_allclose(field[y, x, 0], model(q), rtol=0, atol=.001)

    def test_50m_shading_error_decreases_without_height_error_increase(self):
        for region in ('fuji', 'yari', 'tsukuba'):
            info = json.loads((ROOT/region/'50-diagnostics.json').read_text())
            before = info['previous-radius']; after = info['50-lpf-smooth']
            self.assertLess(after['highFrequencyErrorSlopeRms'], before['highFrequencyErrorSlopeRms']*.35)
            self.assertLessEqual(after['targetRmse'], before['targetRmse']*1.01)


if __name__ == '__main__': unittest.main()
