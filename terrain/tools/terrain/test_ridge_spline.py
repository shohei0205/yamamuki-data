import json
import sqlite3
import unittest
import zlib
import numpy as np
from ridge_spline import design,fit,SPAN


class RidgeSplineTests(unittest.TestCase):
    def test_basis_preserves_plane(self):
        rng=np.random.default_rng(14);xy=np.vstack((rng.uniform(-16000,16000,(100,2)),[[-16000,-16000],[16000,16000]]))
        n=16;axis=(np.arange(n+5)-2)*SPAN/n-SPAN/2;x,y=np.meshgrid(axis,axis)
        a=design(xy,n);np.testing.assert_allclose(np.asarray(a.sum(axis=1)).ravel(),1,atol=1e-12)
        np.testing.assert_allclose(a@(100+x*.02-y*.01).ravel(),100+xy[:,0]*.02-xy[:,1]*.01,atol=1e-9)

    def test_gradient_continues_across_knots(self):
        rng=np.random.default_rng(24);n=16;c=rng.normal(1000,100,(n+5)**2);step=SPAN/n
        def derivative(x):
            d=.001*step
            return float((design([[x+d,1234]],n)@c-design([[x-d,1234]],n)@c)[0]/(2*d))
        for x in [-12000,-2000,0,6000,14000]:
            self.assertLess(abs(derivative(x-.0001*step)-derivative(x+.0001*step)),.0002)

    def test_fit_does_not_use_faces(self):
        axis=np.linspace(-16000,16000,13);x,y=np.meshgrid(axis,axis);h=1000+x*.002-y*.001
        vertices=np.column_stack((x.ravel(),y.ravel(),h.ravel())).tolist()
        m=dict(vertices=vertices,corners=[0,12,156,168,84],info=dict(storedBytes=1,quality='test'),faces=[])
        info,b=fit(m,8);m['faces']=[[0,0,0]];other,bb=fit(m,8)
        self.assertEqual(b,bb);self.assertLess(info['peakError'],.001);self.assertLess(info['rmse'],.01)

    def test_saved_surfaces_preserve_protected_heights(self):
        with sqlite3.connect('build/ridge-surface-half-budget-v2/ridge.sqlite3') as source,sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:
            rows=db.execute('SELECT region,n,info,coefficients FROM surfaces').fetchall();self.assertEqual(len(rows),9)
            for region,n,raw,blob in rows:
                info=json.loads(raw);m=json.loads(zlib.decompress(source.execute('SELECT model FROM surfaces WHERE region=?',(region,)).fetchone()[0]));v=np.array(m['vertices']);coefficients=np.frombuffer(zlib.decompress(blob),'<f4')
                self.assertEqual(len(coefficients),(n+5)**2);self.assertEqual(info['solverStop'],2)
                p=v[m['corners']];np.testing.assert_allclose(design(p[:,:2],n)@coefficients,p[:,2],atol=.01)


if __name__=='__main__':unittest.main()
