import unittest
import numpy as np
from scipy.spatial import Delaunay
from weighted_mesh_allocation import mixed_limits, allocation_score, split_mesh, broad_shape_limits, landform_limits, movable_priority, triangulate_clipped_polygon


class AllocationTests(unittest.TestCase):
    def test_clipped_rounding_does_not_reverse_small_concave_ear(self):
        p=np.array([[0,0],[4,0],[4,4],[2,3.999],[0,4]],dtype=float)
        f=np.asarray(triangulate_clipped_polygon(list(range(5)),p))
        t=p[f];area=np.linalg.det(np.stack([t[:,1]-t[:,0],t[:,2]-t[:,0]],axis=2))/2
        self.assertTrue((area>0).all())
        self.assertAlmostEqual(float(area.sum()),15.998)

    def test_movable_maximum_does_not_hide_one_peak_or_select_fixed_edges(self):
        limits=np.full((20,20),10.);valid=np.ones((20,20),dtype=bool)
        one_peak=np.zeros((20,20));one_peak[10,10]=100.;one_peak[0,:]=10000.
        widespread=np.full((20,20),20.)
        self.assertGreater(movable_priority(one_peak,limits,valid),movable_priority(widespread,limits,valid))
        valid[10,10]=False
        self.assertEqual(movable_priority(one_peak,limits,valid),0.)
        self.assertEqual(movable_priority(one_peak,limits,np.zeros_like(valid)),0.)

    def test_landform_region_score_is_capped_and_does_not_dilute_one_mountain(self):
        valid=np.ones((100,100),dtype=bool);error=np.zeros((100,100));error[40:60,40:60]=50
        weight=np.zeros_like(error);weight[40:60,40:60]=1;limits=np.full_like(error,20.)
        base=allocation_score(error,limits,valid,shape_limits=limits)
        old=allocation_score(error,limits,valid,weight,shape_limits=limits)
        new=allocation_score(error,limits,valid,weight,shape_limits=limits,feature_balance=True)
        self.assertGreaterEqual(new,old);self.assertLessEqual(new,3*old+1e-12)
        self.assertEqual(allocation_score(np.zeros_like(error),limits,valid,weight,shape_limits=limits,feature_balance=True),0)

    def test_landform_gain_is_bounded_and_spreads_over_mountain(self):
        axis=np.arange(-32000,32001,250.);xx,yy=np.meshgrid(axis,axis)
        hill=1000*np.exp(-(xx*xx+yy*yy)/(2*4000**2));valid=np.ones_like(hill,dtype=bool)
        relative=np.full_like(hill,50.);limits,weight=landform_limits(hill,valid,250.,relative)
        gain=mixed_limits(relative)/limits
        self.assertGreaterEqual(float(gain.min()),1.-1e-6);self.assertLessEqual(float(gain.max()),3.5+1e-6)
        annulus=(xx*xx+yy*yy>2000**2)&(xx*xx+yy*yy<5000**2)
        self.assertGreater(float(gain[annulus].mean()),2.)
        self.assertLess(float(np.abs(np.diff(weight,axis=0)).max()),.2)
        plain,w=landform_limits(xx*.03+yy*.02,valid,250.,relative)
        center=(np.abs(xx)<4000)&(np.abs(yy)<4000)
        self.assertLess(float(w[center].max()),1e-4)
        np.testing.assert_allclose(plain[center],50.,atol=.01)
        noise,w=landform_limits(np.sin(xx/100)*.5,valid,250.,relative)
        np.testing.assert_allclose(noise,50.,atol=.01)

    def test_shape_weight_is_broad_and_ignores_flat_noise(self):
        axis=np.arange(-6400,6401,100.);xx,yy=np.meshgrid(axis,axis)
        hill=300*np.exp(-(xx*xx+yy*yy)/(2*1800**2));valid=np.ones_like(hill,dtype=bool)
        limits,weight=broad_shape_limits(hill,valid,100.)
        annulus=(xx*xx+yy*yy>1200**2)&(xx*xx+yy*yy<2400**2)
        self.assertLess(float(limits[annulus].mean()),30.)
        self.assertGreater(float(weight[annulus].mean()),.5)
        self.assertLess(float(np.abs(np.diff(weight,axis=0)).max()),.12)
        plain,presence=broad_shape_limits(np.sin(xx/100)*.5,valid,100.)
        np.testing.assert_allclose(plain,50.,atol=1e-4)
        self.assertEqual(float(presence.max()),0.)
        self.assertEqual(allocation_score(np.full_like(hill,1.5),limits,valid,weight,shape_limits=limits),0.)

    def test_shape_weight_is_height_offset_invariant_and_masks_missing(self):
        axis=np.arange(-3200,3201,100.);xx,yy=np.meshgrid(axis,axis)
        hill=500*np.exp(-(xx*xx+yy*yy)/(2*1500**2));valid=np.ones_like(hill,dtype=bool);valid[:,:10]=False
        first,w=broad_shape_limits(hill,valid,100.);second,v=broad_shape_limits(hill+3000,valid,100.)
        np.testing.assert_allclose(first,second,atol=.001)
        np.testing.assert_allclose(w,v,atol=.0001)
        self.assertTrue(np.isfinite(first).all());self.assertTrue((first[~valid]==50).all())

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
