"""生成・復元の支持半径と適合評価が一致することを実データで検証する。"""
import json
import unittest
from pathlib import Path
import numpy as np
from scipy.stats import qmc
from free_mls import operator
from mls_error_quality import peak_samples, requirements
from mls_lpf_dem import FilteredDem
from mls_quant_codec import decode_quant
from mls_smooth_radius import MODE, model_from_metadata

ROOT = Path('build/mls-smooth-refit-v1')
VIEW = Path('build/mls-generated-preview/dem-regions')


class RefitResultsTests(unittest.TestCase):
    def test_same_basis_at_generation_reconstruction_and_preview(self):
        for region in ('fuji', 'yari', 'tsukuba'):
            for error in (50, 100, 200, 400, 800, 1600, 3200):
                kind = f'{error}-lpf-refit'
                folder = ROOT/region/kind
                info = json.loads((folder/'info.json').read_text())
                p = decode_quant((folder/'model.mlq1').read_bytes())
                old = decode_quant((Path('build/mls-lpf-dem-v2')/region/f'{error}-lpf-balanced/model.mlq1').read_bytes())
                np.testing.assert_array_equal(p[:, :2], old[:, :2])
                self.assertEqual(info['supportMode'], MODE)
                self.assertTrue(info['fittingRepeated'])
                self.assertTrue(info['sameSupportAtFitAndReconstruction'])
                self.assertFalse(info['additionalDisplaySmoothing'])
                self.assertEqual(info['samples'], 11222500)
                self.assertEqual(info['missing'], 0)
                self.assertEqual(info['bytes'], (folder/'model.mlq1').stat().st_size)
                model = model_from_metadata(p, info)
                q = np.random.default_rng(999).uniform(-15000, 15000, (64, 2))
                training_basis = operator(p[:, :2], q, model.support, curvature=1e-5,
                                          tree=model.support.tree)@p[:, 2]
                np.testing.assert_allclose(model(q), training_basis, atol=1e-6, rtol=1e-9)
                n = info['n']; x = np.array([0, 150, 500, n]); y = np.array([n, 700, 40, 0])
                for span in (8000, 32000):
                    h = np.memmap(VIEW/region/f'{kind}-{span}.bin', dtype='<f4', mode='r', shape=(n+1, n+1, 3))
                    q = np.column_stack(((x/n-.5)*span, (y/n-.5)*span))
                    np.testing.assert_allclose(h[y, x, 0], model(q), atol=.001, rtol=0)

    def test_saved_validation_is_measured_with_same_smooth_model(self):
        v = np.vstack(((qmc.Sobol(2, scramble=True, seed=29904).random_base2(15)-.5)*32000,
                       peak_samples(14, 29935, 1000)))
        for region in ('fuji', 'yari', 'tsukuba'):
            folder = ROOT/region/'50-lpf-refit'
            info = json.loads((folder/'info.json').read_text())
            p = decode_quant((folder/'model.mlq1').read_bytes())
            model = model_from_metadata(p, info)
            target = FilteredDem(region, info['sigmaMeters'])
            self.assertEqual(target.digest, info['filterHaloManifestSha256'])
            measured = requirements(model(v)-target.sample(v), v, 50)
            for key in ('measuredMaxError', 'measuredPeakMaxError', 'measuredRmse'):
                self.assertAlmostEqual(measured[key], info['validation'][key], delta=1e-5)


if __name__ == '__main__': unittest.main()
