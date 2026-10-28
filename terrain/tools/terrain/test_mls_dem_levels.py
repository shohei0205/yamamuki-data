"""3方式の元DEM基準、残差の親参照、全元画素評価を検証する。"""
import hashlib
import json
import unittest
from pathlib import Path
import numpy as np
from mls_dem_levels import ROOT, predictor
from mls_generated_stable import DirectSupport, generate_linear
from mls_high_precision import evaluate
from mls_quant_codec import decode_quant
from mls_residual_curve import decode
from raw_dem_reference import RawDem, center_for


class DemLevelsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.reports = json.loads((ROOT/'summary.json').read_text())

    def test_all_variants_use_same_dem_and_full_native_audit(self):
        digests = set()
        for kind, info in self.reports.items():
            self.assertEqual(info['reference'], 'original-PNG-DEM', kind)
            self.assertEqual(info['samples'], 11222500, kind)
            self.assertEqual(info['missing'], 0, kind)
            self.assertEqual(info['converged'], info['measuredMaxError'] <= info['targetError']
                             and info['measuredPeakMaxError'] <= info['targetError']/2)
            digests.add(info['sourceManifestSha256'])
        self.assertEqual(len(digests), 1)

    def test_residual_uses_published_25m_parent_and_all_generated_xy(self):
        parent_blob = (ROOT/'25-independent/model.mlq1').read_bytes()
        parent = decode_quant(parent_blob)
        before = parent.copy()
        xy, coefficients = decode(parent, (ROOT/'12.5-residual/residual.mlr1').read_bytes())
        info = self.reports['12.5-residual']
        self.assertEqual(info['parentSha256'], hashlib.sha256(parent_blob).hexdigest())
        np.testing.assert_array_equal(xy, generate_linear(parent, 1)[:, :2])
        np.testing.assert_array_equal(parent, before)
        self.assertEqual(info['residualControls'], len(coefficients))
        self.assertEqual(info['totalBytes'], len(parent_blob)+info['bytes'])
        self.assertTrue(np.isfinite(coefficients).all())

    def test_worst_point_restores_reported_error(self):
        source = RawDem('build/terrain-cache', center_for('yari'))
        parent = decode_quant((ROOT/'25-independent/model.mlq1').read_bytes())
        for kind, info in self.reports.items():
            q = np.array([info['worst']['xy']])
            if kind == '12.5-residual':
                xy, z = decode(parent, (ROOT/kind/'residual.mlr1').read_bytes())
                actual = predictor(parent)(q)+evaluate(xy, z, q, DirectSupport(xy))
            else:
                actual = predictor(decode_quant((ROOT/kind/'model.mlq1').read_bytes()))(q)
            self.assertAlmostEqual(float(abs(actual-source.sample(q))[0]), info['measuredMaxError'], delta=.001)


if __name__ == '__main__':
    unittest.main()
