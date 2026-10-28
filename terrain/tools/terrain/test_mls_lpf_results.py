"""LPF比較データの独立評価・点配置・表示用の復元を検証する。"""
import json
import unittest
from pathlib import Path
import numpy as np
from mls_fast_eval import FastModel
from mls_lpf_dem import FilteredDem
from mls_quant_codec import decode_quant
from raw_dem_reference import STEP

ROOT = Path('build/mls-lpf-dem-v2')
VIEW = Path('build/mls-generated-preview/dem-regions')


class LpfResultsTests(unittest.TestCase):
    def test_all_coarse_levels_have_two_separate_full_dem_metrics(self):
        for region in ('fuji', 'yari', 'tsukuba'):
            for error in (50, 100, 200, 400, 800, 1600, 3200):
                kind = f'{error}-lpf-balanced'
                folder = ROOT/region/kind
                info = json.loads((folder/'info.json').read_text())
                baseline = Path('build/mls-dem-regions-v1')/region/f'{error}-independent'
                before = decode_quant((baseline/'model.mlq1').read_bytes())
                p = decode_quant((folder/'model.mlq1').read_bytes())
                np.testing.assert_array_equal(p[:, :2], before[:, :2])
                self.assertEqual(info['samples'], 11222500)
                self.assertEqual(info['missing'], 0)
                self.assertEqual(info['fittingReference'], 'Gaussian-filtered-PNG-DEM')
                self.assertEqual(info['reference'], 'original-PNG-DEM')
                self.assertEqual(info['curvatureFactor'], 1.)
                self.assertAlmostEqual(info['sigmaMeters'], .25*32000/np.sqrt(len(p)))
                self.assertEqual(info['bytes'], (folder/'model.mlq1').stat().st_size)
                self.assertEqual(info['converged'], info['measuredMaxError'] <= error and info['measuredPeakMaxError'] <= error/2)
                filtered = info['filteredTargetValidation']
                self.assertEqual(filtered['converged'], filtered['measuredMaxError'] <= error and filtered['measuredPeakMaxError'] <= error/2)
                model = FastModel(p[:, :2], p[:, 2])
                for span in (8000, 32000):
                    n = info['n']
                    field = np.memmap(VIEW/region/f'{kind}-{span}.bin', dtype='<f4', mode='r', shape=(n+1, n+1, 3))
                    x = np.array([0, 100, 400, n]); y = np.array([n, 50, 700, 0])
                    q = np.column_stack(((x/n-.5)*span, (y/n-.5)*span))
                    np.testing.assert_allclose(field[y, x, 0], model(q), atol=.001, rtol=0)

    def test_50m_fine_shading_error_is_lower_without_extra_control_points(self):
        for region in ('fuji', 'yari', 'tsukuba'):
            rows = json.loads((ROOT/region/'50-diagnostics.json').read_text())
            old = rows['original']; new = rows['50-lpf-balanced']
            self.assertLess(new['highFrequencyErrorSlopeRms'], old['highFrequencyErrorSlopeRms']*.5)
            self.assertLess(new['targetRmse'], old['targetRmse'])

    def test_filter_has_full_halo_at_region_edge(self):
        source = FilteredDem('fuji', .25*32000/np.sqrt(8192))
        q = np.array([[-16000, -16000], [16000, 16000]])
        pixels = (q+source.center)/STEP-.5-[source.left, source.top]
        margin = np.minimum(pixels, np.array(source.grid.shape[::-1])-1-pixels)
        self.assertGreater(float(margin.min()), source.sigma_meters/STEP*4)
        self.assertTrue(np.isfinite(source.sample(q)).all())


if __name__ == '__main__': unittest.main()
