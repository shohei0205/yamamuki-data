import unittest
import numpy as np
from scipy.spatial import Delaunay
from weighted_mesh_allocation import mixed_limits, allocation_score, split_mesh


class AllocationTests(unittest.TestCase):
    def test_absolute_error_keeps_large_relief_competitive(self):
        valid = np.ones((8, 8), dtype=bool)
        high = allocation_score(np.full((8, 8), 100.), np.full((8, 8), 100.), valid)
        low = allocation_score(np.full((8, 8), 10.), np.full((8, 8), 10.), valid)
        self.assertGreater(high, low)
        self.assertGreaterEqual(mixed_limits(np.array([.01, 10., 100.])).min(), 40.)

    def test_area_and_flat_noise(self):
        valid = np.ones((10, 10), dtype=bool)
        e = np.zeros((10, 10)); e[0, 0] = 100
        broad = e.copy(); broad[1:, :] = 100
        self.assertGreater(allocation_score(broad, np.full_like(e, 50), valid), allocation_score(e, np.full_like(e, 50), valid))
        self.assertEqual(allocation_score(np.ones_like(e), np.ones_like(e), valid), 0)

    def test_isolated_mountain_is_not_diluted_by_plain(self):
        valid = np.ones((100, 100), dtype=bool)
        error = np.zeros((100, 100)); error[40:60, 40:60] = 80
        terrain = np.zeros_like(error); terrain[40:60, 40:60] = 1
        limits = np.full_like(error, 50)
        base = allocation_score(error, limits, valid)
        adjusted = allocation_score(error, limits, valid, terrain)
        self.assertGreater(adjusted, base * 2)
        self.assertLessEqual(adjusted, base * 6 + 1e-12)
        self.assertEqual(allocation_score(error, limits, valid, np.zeros_like(error)), base)
        self.assertEqual(allocation_score(np.ones_like(error), limits, valid, terrain), 0)

    def test_split_matches_all_seams_and_preserves_plane(self):
        rng = np.random.default_rng(5)
        xy = np.vstack([[[0, 0], [32000, 0], [32000, 32000], [0, 32000]], rng.uniform(0, 32000, (80, 2))])
        p = np.column_stack([xy, 100 + .03 * xy[:, 0] + .07 * xy[:, 1]])
        f = Delaunay(xy).simplices
        pieces = split_mesh(p, f)
        self.assertEqual(len(pieces), 16)
        for (x, y), tile in pieces.items():
            q = tile['points']; tri = q[tile['faces'], :2]
            area = np.linalg.det(np.stack([tri[:, 1] - tri[:, 0], tri[:, 2] - tri[:, 0]], axis=2)) / 2
            self.assertTrue((area > 0).all())
            self.assertAlmostEqual(area.sum(), 8000 ** 2, delta=.02)
            expected = 100 + .03 * (q[:, 0] + x * 8000) + .07 * (q[:, 1] + y * 8000)
            self.assertLess(np.abs(q[:, 2] - expected).max(), .002)
            for dx, dy, axis in [(1, 0, 0), (0, 1, 1)]:
                if (x + dx, y + dy) not in pieces: continue
                other = pieces[x + dx, y + dy]['points']
                left = {tuple(v[[1-axis, 2]]) for v in q if v[axis] == 8000}
                right = {tuple(v[[1-axis, 2]]) for v in other if v[axis] == 0}
                self.assertEqual(left, right)

    def test_128km_domain_splits_into_256_tiles(self):
        xy=np.array([[0,0],[128000,0],[128000,128000],[0,128000]],dtype=float)
        p=np.column_stack([xy,100+.01*xy[:,0]+.02*xy[:,1]])
        parts=split_mesh(p,np.array([[0,1,2],[0,2,3]]),width=128000.)
        self.assertEqual(len(parts),256)
        for tile in parts.values():
            q=tile['points'][tile['faces'],:2]
            area=np.linalg.det(np.stack([q[:,1]-q[:,0],q[:,2]-q[:,0]],axis=2))/2
            self.assertAlmostEqual(area.sum(),8000**2,delta=.02)


if __name__ == '__main__': unittest.main()
