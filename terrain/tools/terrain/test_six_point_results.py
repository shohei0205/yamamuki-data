"""狭い範囲でも生成・復元・描画が同一の二次MLSであることを検証する。"""
import json
import unittest
from pathlib import Path
import numpy as np
from free_mls import operator
from mls_quant_codec import decode_quant
from mls_smooth_radius import SIX_MODE, model_from_metadata


class SixResultsTests(unittest.TestCase):
    def test_models_use_narrow_support_and_same_training_basis(self):
        for region in ('fuji', 'yari', 'tsukuba'):
            folder = Path('build/mls-six-point-v3')/region/'50-six-trained'
            info = json.loads((folder/'info.json').read_text())
            p = decode_quant((folder/'model.mlq1').read_bytes())
            source = decode_quant((Path('build/mls-smooth-refit-v1')/region/'50-lpf-refit/model.mlq1').read_bytes())
            np.testing.assert_array_equal(p[:, :2], source[:, :2])
            self.assertEqual(info['supportMode'], SIX_MODE)
            self.assertEqual(info['generationSlopeWeight'], 2.)
            self.assertEqual(info['selectionObjective'], 'normalized-height-and-slope-rms')
            self.assertFalse(info['additionalDisplaySmoothing'])
            self.assertEqual(info['samples'], 11222500)
            model = model_from_metadata(p, info)
            q = np.random.default_rng(771).uniform(-16000, 16000, (20000, 2))
            counts = model.support.tree.query_ball_point(q, model.support(q), return_length=True)
            self.assertGreaterEqual(int(counts.min()), 6)
            self.assertLessEqual(float(np.median(counts)), 10)
            basis = operator(p[:, :2], q[:100], model.support, curvature=1e-5, tree=model.support.tree)
            np.testing.assert_allclose(model(q[:100]), basis@p[:, 2], atol=1e-6)
            n = info['n']; x = np.array([0, 300, 600, n]); y = np.array([n, 500, 50, 0])
            for span in (8000, 32000):
                field = np.memmap(Path('build/mls-generated-preview/dem-regions')/region/f'50-six-trained-{span}.bin', dtype='<f4', mode='r', shape=(n+1, n+1, 3))
                q = np.column_stack(((x/n-.5)*span, (y/n-.5)*span))
                np.testing.assert_allclose(field[y, x, 0], model(q), atol=.001, rtol=0)


if __name__ == '__main__': unittest.main()
