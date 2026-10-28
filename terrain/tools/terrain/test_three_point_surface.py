"""3点局所曲面の基底・傾き保存・連続性を確認する。"""
import unittest
import numpy as np
from three_point_surface import ThreePointModel, CubicHeightBasis, encode, decode
from mls_dem_regions import canonical


class ThreePointTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        xy = np.random.default_rng(9).uniform(-16000, 16000, (128, 2))
        xy[:4] = [[-16000, -16000], [-16000, 16000], [16000, -16000], [16000, 16000]]
        cls.xy = canonical(xy)

    def test_three_height_coefficients_match_cubic_evaluation(self):
        z = np.random.default_rng(8).normal(size=len(self.xy))
        q = np.random.default_rng(81).uniform(-15000, 15000, (2000, 2))
        a = CubicHeightBasis(self.xy).operator(q)
        self.assertTrue(np.all(np.diff(a.indptr) == 3))
        model = ThreePointModel(self.xy, z, np.zeros((len(z), 2)))
        np.testing.assert_allclose(a@z, model(q), atol=1e-9)

    def test_saved_slopes_reproduce_sloping_plane(self):
        z = 500+self.xy[:, 0]*.01-self.xy[:, 1]*.02
        grad = np.tile([.01, -.02], (len(z), 1))
        p, g = decode(encode(np.column_stack((self.xy, z)), grad, .125))
        self.assertLessEqual(float(abs(g-grad).max()), 1/65536)
        np.testing.assert_allclose(ThreePointModel(self.xy, z, grad)(np.array([[40., 300.]])), 494.4, atol=1e-8)
        np.testing.assert_array_equal(p[:, :2], self.xy)

    def test_cubic_gradients_are_continuous_across_shared_edges(self):
        rng = np.random.default_rng(2)
        model = ThreePointModel(self.xy, rng.normal(size=len(self.xy))*10,
                                rng.normal(size=(len(self.xy), 2))*.01)
        tri = model.tri
        i, j = next((i, j) for i, row in enumerate(tri.neighbors) for j in row if j >= 0)
        edge = np.intersect1d(tri.simplices[i], tri.simplices[j])
        middle = self.xy[edge].mean(axis=0); d = self.xy[edge[1]]-self.xy[edge[0]]
        normal = np.array([-d[1], d[0]])/np.linalg.norm(d)
        eps = .001
        left, center, right = model(np.array([middle-normal*eps, middle, middle+normal*eps]))
        self.assertAlmostEqual((center-left)/eps, (right-center)/eps, delta=1e-5)


if __name__ == '__main__': unittest.main()
