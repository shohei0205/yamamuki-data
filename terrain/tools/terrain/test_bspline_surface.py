"""階層曲面の連続性、展開、保存復元を確認する。"""
import json
import os
import sqlite3
import unittest
from pathlib import Path

import numpy as np
from scipy.ndimage import spline_filter
from bspline_surface import HALO, SPAN, evaluate, flatten, load


class SurfaceTests(unittest.TestCase):
    def test_plane_and_derivatives_across_knots(self):
        n=32
        axis=(np.arange(n+1+2*HALO)-HALO)*SPAN/n-SPAN/2
        x,y=np.meshgrid(axis,axis)
        c=spline_filter(1000+.02*x-.03*y,order=3,mode='mirror')
        q=np.linspace(-15000,15000,301)
        np.testing.assert_allclose(evaluate([(n,c)],q,q*.7),1000+.02*q-.03*q*.7,atol=.0001)
        for knot in [-8000,0,8000]:
            eps=.01
            h=evaluate([(n,c)],np.array([knot-eps,knot,knot+eps]),123.)
            self.assertLess(abs((h[1]-h[0])/eps-(h[2]-h[1])/eps),1e-6)

    def test_sparse_local_control_has_compact_support(self):
        n=32;c=np.zeros((n+1+2*HALO,)*2)
        c[HALO+n//2,HALO+n//2]=100
        self.assertGreater(evaluate([(n,c)],0,0),0)
        self.assertEqual(float(evaluate([(n,c)],3000,0)),0)
        self.assertEqual(float(evaluate([(n,c)],0,3000)),0)

    def test_hierarchy_flattening_and_gradient_bounds(self):
        rng=np.random.default_rng(52)
        layers=[]
        for n in [8,16,32]:
            c=rng.normal(size=(n+1+2*HALO,)*2)*10
            layers.append((n,c))
        dense=flatten(layers)
        x=rng.uniform(-15000,15000,2048);y=rng.uniform(-15000,15000,2048)
        np.testing.assert_allclose(evaluate([(32,dense)],x,y),evaluate(layers,x,y),atol=.001)
        for axis in [0,1]:
            dx=.1 if axis==1 else 0;dy=.1 if axis==0 else 0
            derivative=(evaluate([(32,dense)],x+dx,y+dy)-evaluate([(32,dense)],x-dx,y-dy))/.2
            bound=np.max(abs(np.diff(dense,axis=axis)))/(SPAN/32)
            self.assertLessEqual(float(np.max(abs(derivative))),bound+1e-5)

    def test_generated_three_regions_seven_qualities(self):
        path=Path(os.environ.get('BSPLINE_DB','build/bspline-surface-prototype-v3/spline.sqlite3'))
        fixtures=[]
        with sqlite3.connect(path.resolve().as_uri()+'?mode=ro',uri=True) as db:
            self.assertEqual(db.execute('SELECT count(*) FROM layers').fetchone()[0],21)
            rng=np.random.default_rng(11)
            for region,n,raw in db.execute('SELECT region,n,info FROM layers'):
                report=json.loads(raw);info,layers=load(db,region,n)
                self.assertEqual(report,info)
                self.assertEqual(info['cumulativeCoefficients'],sum(np.count_nonzero(c) for _,c in layers))
                dense=flatten(layers)
                self.assertTrue(np.isfinite(dense).all())
                x=rng.uniform(-16000,16000,1024);y=rng.uniform(-16000,16000,1024)
                np.testing.assert_allclose(evaluate([(n,dense)],x,y),evaluate(layers,x,y),atol=.002)
                if n in [128,512]:
                    step=SPAN/n
                    # 区間境界の両側でGPUの整数部・小数部の評価が食い違わないか照合する。
                    edge=np.repeat([-step,0,step],7)+np.tile([-.003,-.001,-.0005,0,.0005,.001,.003],3)
                    x=np.r_[x[:12],edge,np.full(len(edge),3333.333)]
                    y=np.r_[y[:12],np.full(len(edge),7037.9697),edge];eps=.1
                    # 差分の参照値はfloat32の高さ丸めを避けて倍精度で評価する。
                    dense=dense.astype(np.float64)
                    h=evaluate([(n,dense)],x,y)
                    dx=(evaluate([(n,dense)],x+eps,y)-evaluate([(n,dense)],x-eps,y))/(2*eps)
                    dy=(evaluate([(n,dense)],x,y+eps)-evaluate([(n,dense)],x,y-eps))/(2*eps)
                    fixtures.append(dict(region=region,n=n,points=np.column_stack([x,y,h,dx,dy]).tolist()))
        (path.parent/'gpu-field-fixture.json').write_text(json.dumps(fixtures),encoding='utf-8')


if __name__=='__main__':unittest.main()
