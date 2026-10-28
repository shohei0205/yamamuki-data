"""3地域・全段階の元DEM評価と圧縮参照を検証する。"""
import hashlib
import json
import unittest
from pathlib import Path
import numpy as np
from mls_dem_regions import ROOT, VIEW, model_from_disk
from mls_quant_codec import decode_quant
from raw_dem_reference import RawDem, center_for


class DemRegionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.reports = json.loads((ROOT/'summary.json').read_text())

    def test_all_requested_regions_and_qualities_have_native_audits(self):
        expected = {f'{e}-independent' for e in ('12.5','25','50','100','200','400','800','1600','3200')}
        expected.add('12.5-residual')
        self.assertEqual(set(self.reports), {'fuji', 'yari', 'tsukuba'})
        for region, rows in self.reports.items():
            source = RawDem('build/terrain-cache', center_for(region))
            samples = sum(len(h) for _, h, _ in source.native_rows(128))
            self.assertEqual(set(rows), expected)
            for kind, info in rows.items():
                self.assertEqual(info['sourceManifestSha256'], source.digest)
                self.assertEqual(info['reference'], 'original-PNG-DEM')
                self.assertEqual(info['samples'], samples)
                self.assertEqual(info['missing'], 0)
                self.assertEqual(info['converged'], info['measuredMaxError'] <= info['targetError'] and
                                 info['measuredPeakMaxError'] <= info['targetError']/2)
                for span in (32000, 8000):
                    size = (info['n']+1)**2*3*4
                    self.assertEqual((VIEW/region/f'{kind}-{span}.bin').stat().st_size, size)

    def test_exact_saved_models_restore_reported_worst_errors(self):
        for region, rows in self.reports.items():
            source = RawDem('build/terrain-cache', center_for(region))
            parent_blob = (ROOT/region/'25-independent/model.mlq1').read_bytes()
            parent = decode_quant(parent_blob)
            for kind, info in rows.items():
                model = model_from_disk(ROOT/region/kind, parent if kind.endswith('residual') else None)
                q = np.array([info['worst']['xy']])
                error = float(abs(model(q)-source.sample(q))[0])
                self.assertAlmostEqual(error, info['measuredMaxError'], delta=.001)
                path = ROOT/region/kind/('residual.mlr1' if kind.endswith('residual') else 'model.mlq1')
                self.assertEqual(path.stat().st_size, info['bytes'])
                if kind.endswith('residual'):
                    self.assertEqual(info['parentSha256'], hashlib.sha256(parent_blob).hexdigest())
                    self.assertEqual(info['totalBytes'], info['bytes']+len(parent_blob))


if __name__ == '__main__':
    unittest.main()
