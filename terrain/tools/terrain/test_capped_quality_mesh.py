import unittest
import numpy as np
from capped_quality_mesh import build_to_quality
from refine_shared_quality import triangulate
from compare_shared_allocation import residual


def fake_grow(region,mode,target,force):
    assert not force
    grid=region['grid'];p=np.array([[x,y,grid[y,x]] for x,y in region['seed']],float)
    while True:
        f=triangulate(p);e=residual(p,f,grid);v=np.abs(e)/region['shape_limits']
        if v.max()<=1 or len(p)>=target:break
        y,x=np.unravel_index(v.argmax(),v.shape)
        p=np.vstack([p,[x,y,grid[y,x]]])
    return dict(p=p,f=f,maximum=float(np.abs(e).max()),rmse=float(np.sqrt((e*e).mean())))


def region(grid,limit=1):
    n=len(grid);return dict(grid=grid,seed=[[0,0],[n-1,0],[n-1,n-1],[0,n-1]],limits=np.full_like(grid,limit),shape_limits=np.full_like(grid,limit),valid=np.ones_like(grid,bool))


class CappedTests(unittest.TestCase):
    def test_flat_stops_at_four_without_filling_large_cap(self):
        mesh,r=build_to_quality(region(np.zeros((9,9))),100,grow_mesh=fake_grow)
        self.assertEqual(len(mesh['p']),4);self.assertEqual(r['unusedVertices'],96)
        self.assertEqual(r['stopReason'],'quality-reached');self.assertTrue(r['qualityReached'])

    def test_error_and_angle_both_required(self):
        g=np.zeros((9,9));g[4,4]=100
        mesh,r=build_to_quality(region(g),5,minimum=10,grow_mesh=fake_grow)
        self.assertEqual(r['stopReason'],'vertex-cap');self.assertFalse(r['errorReached'])
        self.assertLessEqual(len(mesh['p']),5)

    def test_error_alone_does_not_stop_with_bad_angles(self):
        v=region(np.zeros((9,9)));v['seed'].append([4,1])
        mesh,r=build_to_quality(v,5,minimum=30,grow_mesh=fake_grow)
        self.assertTrue(r['errorReached']);self.assertFalse(r['angleReached'])
        self.assertEqual(r['stopReason'],'vertex-cap')

    def test_chunked_error_matches_original_raster(self):
        from tin_vertex_optimize import raster
        rng=np.random.default_rng(12);p=np.vstack([[[0,0,0],[16,0,20],[16,16,30],[0,16,5]],np.column_stack([rng.integers(1,16,(12,2)),rng.normal(size=12)])]);p=np.unique(p,axis=0)
        f=triangulate(p);grid=np.zeros((17,17));ids,w=raster(p,f,17)
        expected=(p[f[ids],2]*w).sum(axis=1).reshape(grid.shape)
        np.testing.assert_allclose(residual(p,f,grid),expected,atol=1e-10)

    def test_error_uses_actual_diagonal_for_cocircular_points(self):
        from tin_vertex_optimize import raster
        p=np.array([[0,0,0],[8,0,0],[8,8,10],[0,8,0]],float)
        grid=np.zeros((9,9))
        for f in [np.array([[0,1,2],[0,2,3]]),np.array([[0,1,3],[1,2,3]])]:
            ids,w=raster(p,f,9);expected=(p[f[ids],2]*w).sum(axis=1).reshape(grid.shape)
            np.testing.assert_allclose(residual(p,f,grid),expected,atol=1e-10)

    def test_too_small_cap_rejected(self):
        with self.assertRaises(ValueError):build_to_quality(region(np.zeros((9,9))),3,grow_mesh=fake_grow)


if __name__=='__main__':unittest.main()
