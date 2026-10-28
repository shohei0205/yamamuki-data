"""ネイティブ評価がNumPy版のMLSと同じ曲面を返すことを確かめる。"""
import unittest
import numpy as np
from mls_fast_eval import FastModel
from mls_generated_stable import DirectSupport
from mls_high_precision import evaluate
from mls_quant_codec import decode_quant
from pathlib import Path


class NativeMlsTests(unittest.TestCase):
    def test_sparse_and_dense_models_match(self):
        rng = np.random.default_rng(200)
        for count in (24, 32, 64, 128, 2048):
            xy = rng.uniform(-16000, 16000, (count, 2))
            xy[:4] = [[-16000, -16000], [-16000, 16000], [16000, -16000], [16000, 16000]]
            z = rng.uniform(0, 4000, count)
            q = np.vstack((xy[:4], rng.uniform(-16000, 16000, (1000, 2))))
            expected = evaluate(xy, z, q, DirectSupport(xy))
            actual = FastModel(xy, z)(q)
            np.testing.assert_allclose(actual, expected, rtol=1e-9, atol=1e-6)

    def test_existing_worst_errors_match_after_quantized_restore(self):
        root = Path('build/mls-dem-levels-yari-v1')
        for kind in ('25-independent', '12.5-independent'):
            p = decode_quant((root/kind/'model.mlq1').read_bytes())
            q = np.random.default_rng(92).uniform(-16000, 16000, (4000, 2))
            expected = evaluate(p[:, :2], p[:, 2], q, DirectSupport(p[:, :2]))
            np.testing.assert_allclose(FastModel(p[:, :2], p[:, 2])(q), expected, rtol=1e-9, atol=1e-6)


if __name__ == '__main__':
    unittest.main()
