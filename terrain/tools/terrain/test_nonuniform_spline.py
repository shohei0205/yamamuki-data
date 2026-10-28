"""不等間隔の配置、数値安定性、保存曲面の復元を検証する。"""
import json,sqlite3,unittest,zlib
import numpy as np
from scipy.interpolate import BSpline
from nonuniform_spline import cardinal,fit_surface,knots_for
class NonuniformTests(unittest.TestCase):
    def test_nonuniform_knots_and_partition(self):
        x=np.linspace(-16000,16000,513);density=1+20*np.exp(-((x-3000)/2000)**2);t=knots_for(density,74,x)
        intervals=np.diff(np.unique(t));self.assertGreater(intervals.max()/intervals.min(),2)
        b=BSpline.design_matrix(x,t,3).toarray();np.testing.assert_allclose(b.sum(axis=1),1,atol=1e-12)
        self.assertTrue(np.all(b>=0))
    def test_plane_is_preserved(self):
        x=np.linspace(-16000,16000,129);xx,yy=np.meshgrid(x,x);h=200+.1*xx-.05*yy
        e,tx,ty,c=fit_surface(h,x,30);self.assertLess(e,1e-8)
    def test_all_saved_models_are_stable(self):
        x=np.linspace(-16000,16000,513);u=cardinal(256,x)
        with sqlite3.connect('build/nonuniform-spline-v2/nonuniform.sqlite3') as db:rows=db.execute('SELECT * FROM surfaces').fetchall()
        self.assertEqual(len(rows),15)
        for region,keep,raw,xb,yb,cb in rows:
            info=json.loads(raw);tx=np.frombuffer(zlib.decompress(xb),'<f4');ty=np.frombuffer(zlib.decompress(yb),'<f4');c=np.frombuffer(zlib.decompress(cb),'<f4').reshape(info['controlSide'],-1)
            self.assertEqual(sum(map(len,(xb,yb,cb))),info['cumulativeBytes']);self.assertLess(abs(c).max(),50000)
            h=BSpline.design_matrix(x,ty,3)@c@BSpline.design_matrix(x,tx,3).T
            self.assertTrue(np.isfinite(h).all());self.assertGreater(np.std(np.diff(np.unique(tx))),1)
            self.assertLess(info['displayMax'],10)
if __name__=='__main__':unittest.main()
