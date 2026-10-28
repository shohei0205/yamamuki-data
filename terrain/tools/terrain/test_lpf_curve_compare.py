"""曲面係数・勾配の連続性とXY包含／高さ差分の照合。"""
import os
import sqlite3
import unittest
from pathlib import Path
import numpy as np
from scipy.spatial import Delaunay
from lpf_curve_compare import patches, basis, load_level, relaxed_heights

class CurveTest(unittest.TestCase):
    def test_local_approximation_preserves_plane_and_limits_height_changes(self):
        xx,yy=np.meshgrid(np.arange(20)*10.,np.arange(20)*10.)
        xy=np.column_stack((xx.ravel(),yy.ravel()));plane=2+xy[:,0]*0.2+xy[:,1]*0.3
        np.testing.assert_allclose(relaxed_heights(xy,plane),plane,atol=1e-6)
        noisy=plane+np.random.default_rng(42).normal(size=len(plane))
        fitted=relaxed_heights(xy,noisy,maximum=0.75)
        self.assertLessEqual(float(np.max(abs(fitted-noisy))),0.750001)
        self.assertLess(np.std(fitted-plane),np.std(noisy-plane)*0.8)

    def test_plane_is_reproduced_by_saved_cubic_coefficients(self):
        xy=np.array([[0.,0.],[1000.,0.],[0.,1000.],[1000.,1000.],[350.,450.]])
        tri=Delaunay(xy);h=3+xy[:,0]*0.2+xy[:,1]*0.3
        coef,curve,error=patches(tri,h,np.tile([0.2,0.3],(len(xy),1)))
        self.assertLess(error,0.001)
        np.testing.assert_allclose(curve([[100.,200.],[400.,700.]]),[83.,293.],atol=0.001)
        for f,ids in enumerate(tri.simplices):
            center=xy[ids].mean(0)
            for k in range(3):
                corners=np.array([xy[ids[k]],xy[ids[(k+1)%3]],center])
                bary=np.array([[0.2,0.3,0.5]])
                p=bary@corners
                np.testing.assert_allclose(basis(bary)@coef[f,k],3+p[:,0]*0.2+p[:,1]*0.3,atol=0.001)

    def test_height_and_gradient_are_continuous_across_shared_edges(self):
        xy=np.array([[0.,0.],[1000.,0.],[0.,1000.],[1000.,1000.],[350.,450.]])
        tri=Delaunay(xy);h=100*np.sin(xy[:,0]/700)+50*np.cos(xy[:,1]/400)
        g=np.column_stack([100/700*np.cos(xy[:,0]/700),-50/400*np.sin(xy[:,1]/400)])
        _,curve,_=patches(tri,h,g)
        edges={}
        for ids in tri.simplices:
            for k in range(3):edges.setdefault(tuple(sorted([int(ids[k]),int(ids[(k+1)%3])])),0);edges[tuple(sorted([int(ids[k]),int(ids[(k+1)%3])]))]+=1
        for (a,b),count in edges.items():
            if count!=2:continue
            p=(xy[a]+xy[b])/2;v=xy[b]-xy[a];n=np.array([-v[1],v[0]])/np.linalg.norm(v);eps=0.01
            grad=lambda q:np.array([(curve([q+e])[0]-curve([q-e])[0])/(2*eps) for e in [np.array([eps,0]),np.array([0,eps])]])
            np.testing.assert_allclose(grad(p+n*eps),grad(p-n*eps),atol=1e-4)

    def test_all_generated_levels_keep_xy_and_restore_changed_parent_heights(self):
        path=Path(os.environ.get('LPF_CURVE_DB','build/lpf-curve-prototype/comparison.sqlite3'))
        self.assertTrue(path.exists(),'先に地域比較データを生成してください')
        with sqlite3.connect(path.resolve().as_uri()+'?mode=ro',uri=True) as db:
            changed=0
            for region in ['fuji','yari','tsukuba']:
                old_xy=np.empty((0,2));old_h=np.empty(0)
                for z in range(6,15):
                    info,xy,h,_,faces,coef=load_level(db,region,z)
                    np.testing.assert_array_equal(xy[:len(old_xy)],old_xy)
                    self.assertEqual(info['parentCount'],len(old_xy));self.assertTrue(np.isfinite(h).all())
                    self.assertTrue(np.max(faces)<len(xy));self.assertTrue(np.isfinite(coef).all())
                    if info.get('curveFitGridError') is not None:self.assertLessEqual(info['curveFitGridError'],info['curveFitTolerance'])
                    changed+=int(np.count_nonzero(abs(h[:len(old_h)]-old_h)>0.01))
                    old_xy,old_h=xy,h
            self.assertGreater(changed,0)

    def test_smoothed_data_keep_original_xy_and_bound_total_height_change(self):
        path=Path(os.environ.get('LPF_CURVE_DB','build/lpf-curve-prototype/comparison.sqlite3'))
        with sqlite3.connect(path.resolve().as_uri()+'?mode=ro',uri=True) as db:
            info=db.execute('SELECT info FROM levels LIMIT 1').fetchone()[0]
            if 'relaxed-height-fit' not in info:self.skipTest('平滑化データの照合は別出力に対して実行する')
            with sqlite3.connect(Path('build/lpf-curve-fit-prototype/comparison.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as old:
                for region in ['fuji','yari','tsukuba']:
                    for z in range(6,15):
                        _,xy,h,_,faces,_=load_level(db,region,z)
                        _,old_xy,old_h,_,old_faces,_=load_level(old,region,z)
                        np.testing.assert_array_equal(xy,old_xy);np.testing.assert_array_equal(faces,old_faces)
                        self.assertLess(float(np.max(abs(h-old_h))),12.501)

if __name__=='__main__':unittest.main()
