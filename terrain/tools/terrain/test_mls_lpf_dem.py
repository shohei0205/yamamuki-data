"""LPFの欠損・平面保持と、曲率罰則の平面保持を検証する。"""
import unittest
import numpy as np
from mls_lpf_dem import normalized_lpf, curvature_matrix


class LpfTests(unittest.TestCase):
    def test_missing_is_not_zero_and_constants_are_preserved(self):
        h = np.full((80, 80), 800.)
        h[30:40, 30:40] = np.nan
        filtered = normalized_lpf(h, 3)
        np.testing.assert_array_equal(np.isnan(filtered), np.isnan(h))
        np.testing.assert_allclose(filtered[np.isfinite(h)], 800., atol=1e-10)

    def test_lpf_preserves_interior_sloping_plane(self):
        x, y = np.meshgrid(np.arange(80), np.arange(80))
        h = 300.+x*.5-y*.2
        np.testing.assert_allclose(normalized_lpf(h, 2)[10:-10, 10:-10], h[10:-10, 10:-10], atol=1e-10)

    def test_curvature_penalty_does_not_flatten_sloping_planes(self):
        xy = np.random.default_rng(200).uniform(-16000, 16000, (128, 2))
        penalty = curvature_matrix(xy)
        plane = 1000.+xy[:, 0]*.02-xy[:, 1]*.03
        np.testing.assert_allclose(penalty@plane, 0., atol=1e-6)
        self.assertGreater(float(np.linalg.norm(penalty@np.random.default_rng(8).normal(size=128))), 1.)


if __name__ == '__main__': unittest.main()
