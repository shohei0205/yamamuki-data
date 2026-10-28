"""支持半径の連続性、決定性、XY保持と元実装との互換性を確認する。"""
import unittest
import numpy as np
from mls_smooth_radius import SmoothSupport
from mls_fast_eval import FastModel


class SmoothRadiusTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        rng = np.random.default_rng(901)
        cls.xy = rng.uniform(-16000, 16000, (128, 2))
        cls.xy[:4] = [[-16000, -16000], [-16000, 16000], [16000, -16000], [16000, 16000]]
        cls.support = SmoothSupport(cls.xy)

    def test_deterministic_and_does_not_change_points(self):
        saved = self.xy.copy()
        q = np.array([[0, 0], [-16000, 16000], [16000, -16000]])
        np.testing.assert_array_equal(self.support(q), SmoothSupport(self.xy)(q))
        np.testing.assert_array_equal(self.xy, saved)
        self.assertTrue(np.all(self.support(q) > 0))
        self.assertFalse(self.support.metadata()['radiusFieldStored'])

    def test_derivatives_match_on_both_sides_of_interpolation_knots(self):
        for x in (-8000., 0., 125., 10000.):
            q = np.array([[x, 123.], [x-.001, 123.], [x+.001, 123.]])
            r = self.support(q)
            left, right = (r[0]-r[1])/.001, (r[2]-r[0])/.001
            self.assertAlmostEqual(float(left), float(right), delta=1e-5)

    def test_plane_height_is_still_reproduced(self):
        z = 1000+self.xy[:, 0]*.01-self.xy[:, 1]*.02
        model = FastModel(self.xy, z, support=self.support)
        q = np.random.default_rng(90).uniform(-15000, 15000, (100, 2))
        np.testing.assert_allclose(model(q), 1000+q[:, 0]*.01-q[:, 1]*.02, atol=.001)

    def test_invalid_domain_and_support_rejected(self):
        with self.assertRaises(ValueError): self.support(np.array([[16001., 0.]]))
        with self.assertRaises(ValueError): SmoothSupport(self.xy, halo=1)
        with self.assertRaises(ValueError): FastModel(self.xy+10, np.zeros(len(self.xy)), support=self.support)


if __name__ == '__main__': unittest.main()
