"""頂点最適化の誤差定義と拘束条件のテスト。"""
import unittest
import numpy as np
from scipy.spatial import Delaunay
from tin_vertex_optimize import raster,features,metrics,optimize,locked_vertices
class Tests(unittest.TestCase):
    def test_volume_uses_absolute_height_and_area(self):
        n=17;grid=np.ones((n,n));points=np.array([[0,0,0],[16,0,0],[16,16,0],[0,16,0]],float);faces=np.array([[0,1,2],[0,2,3]]);weights,views=features(grid);r,_,_=metrics(points,faces,grid,weights,views);self.assertAlmostEqual(r['absoluteVolumeM3'],16000**2);self.assertAlmostEqual(r['rmse'],1)
        grid=np.tile(np.linspace(-1,1,n),(n,1));weights,views=features(grid);r,_,_=metrics(points,faces,grid,weights,views);self.assertAlmostEqual(r['absoluteVolumeM3'],16000**2/2)
    def test_all_variants_keep_vertices_and_do_not_worsen_objective(self):
        n=17;y,x=np.indices((n,n));grid=50*np.sin(x/4)*np.sin(y/5)+x*2;coords=np.array([(xx,yy) for yy in range(0,n,4) for xx in range(0,n,4)]);points=np.c_[coords,grid[coords[:,1],coords[:,0]]];faces=Delaunay(coords).simplices;weights,views=features(grid);baseline,_,_=metrics(points,faces,grid,weights,views);locked,_=locked_vertices(points,faces,n)
        for variant in ['z_rms','z_max','xy_rms','xy_max','feature_z','feature_xy','volume_z','volume_xy']:
            with self.subTest(variant=variant):
                result,r,info=optimize(points,faces,grid,variant,weights,views);self.assertEqual(result.shape,points.shape);np.testing.assert_array_equal(result[locked],points[locked]);raster(result,faces,n);key='absoluteVolumeM3' if variant.startswith('volume') else ('maxError' if variant.endswith('max') else ('featureRmse' if variant.startswith('feature') else 'rmse'));self.assertLessEqual(r[key],baseline[key]+1e-5)
if __name__=='__main__':unittest.main()
