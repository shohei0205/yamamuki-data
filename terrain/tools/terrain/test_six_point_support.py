"""狭い支持半径の6点保証・微分連続性と二次MLSの復元を検証する。"""
import unittest
import numpy as np
from mls_smooth_radius import SixPointSupport
from mls_fast_eval import FastModel


class SixPointTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        rng = np.random.default_rng(882)
        cls.xy = rng.uniform(-16000, 16000, (512, 2))
        cls.xy[:4] = [[-16000, -16000], [-16000, 16000], [16000, -16000], [16000, 16000]]
        cls.support = SixPointSupport(cls.xy)

    def test_radius_includes_sixth_point_including_region_corners(self):
        q = np.vstack((self.xy[:4], np.random.default_rng(32).uniform(-16000, 16000, (20000, 2))))
        r = self.support(q)
        d6 = self.support.tree.query(q, k=6)[0][:, -1]
        self.assertTrue(np.all(r >= d6*self.support.expansion-1e-6))
        counts = self.support.tree.query_ball_point(q, r, return_length=True)
        self.assertGreaterEqual(int(counts.min()), 6)
        self.assertLessEqual(float(np.median(counts)), 10)

    def test_radius_gradient_has_no_interpolation_knot_jump(self):
        for x in (0., 31.25, 1000., -5000.):
            eps = .001
            r = self.support(np.array([[x-eps, 12.], [x, 12.], [x+eps, 12.]]))
            self.assertAlmostEqual((r[1]-r[0])/eps, (r[2]-r[1])/eps, delta=.001)

    def test_model_reproduces_sloping_plane(self):
        z = 500+self.xy[:, 0]*.01-self.xy[:, 1]*.03
        model = FastModel(self.xy, z, support=self.support)
        q = np.random.default_rng(27).uniform(-15000, 15000, (500, 2))
        np.testing.assert_allclose(model(q), 500+q[:, 0]*.01-q[:, 1]*.03, atol=.001)


if __name__ == '__main__': unittest.main()
