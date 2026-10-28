import json
import sqlite3
import unittest
import zlib
from pathlib import Path
import numpy as np
from feature_surface import evaluate,fit,SPAN


class FeatureTests(unittest.TestCase):
    def test_analytic_gradient(self):
        base=np.array([120.,50.,-30.]);features=np.array([[133.,-277.,1800.,3200.,.4,1000.],[-3300.,1700.,3500.,900.,-.8,-300.]])
        x=np.linspace(-6000,6000,53);y=x*.3
        h,gx,gy=evaluate(base,features,x,y,True);eps=.01
        np.testing.assert_allclose(gx,(evaluate(base,features,x+eps,y)-evaluate(base,features,x-eps,y))/(2*eps),atol=1e-8)
        np.testing.assert_allclose(gy,(evaluate(base,features,x,y+eps)-evaluate(base,features,x,y-eps))/(2*eps),atol=1e-8)
        self.assertTrue(np.isfinite(h).all())

    def test_fits_free_center_and_flat_region(self):
        axis=np.linspace(-16000,16000,41);x,y=np.meshgrid(axis,axis)
        h=evaluate(np.array([200.,10.,-20.]),np.array([[133.,-277.,1800.,3200.,.4,1000.]]),x,y)
        models=fit(x,y,h,budgets=[1,4])
        _,base,features=models[-1]
        self.assertLess(np.sqrt(np.mean((evaluate(base,features,x,y)-h)**2)),10.)
        self.assertTrue(np.any(np.abs(features[:,:2]/800-np.round(features[:,:2]/800))>.01))
        flat=fit(x,y,100+x*.001,budgets=[1])[-1]
        np.testing.assert_allclose(evaluate(flat[1],flat[2],x,y),100+x*.001,atol=.001)

    def test_generated_storage_and_non_grid_positions(self):
        path=Path('build/feature-surface-prototype-v2/features.sqlite3')
        with sqlite3.connect(path.resolve().as_uri()+'?mode=ro',uri=True) as db:
            self.assertEqual(db.execute('SELECT count(*) FROM surfaces').fetchone()[0],12)
            for region,budget,raw,bb,fb in db.execute('SELECT * FROM surfaces'):
                info=json.loads(raw);base=np.frombuffer(bb,'<f4');features=np.frombuffer(zlib.decompress(fb),'<f4').reshape(-1,6)
                self.assertEqual(features.shape,(budget,6));self.assertEqual(len(base),3)
                self.assertTrue(np.isfinite(features).all());self.assertTrue((features[:,2:4]>=249.99).all())
                self.assertEqual(info['rawBytes'],12+24*budget)
                self.assertEqual(info['storedBytes'],len(bb)+len(fb))
                self.assertTrue(np.any(np.abs(features[:,:2]/250-np.round(features[:,:2]/250))>.001))
                self.assertAlmostEqual(float(evaluate(base,features,0,0)),info['centerHeight'],places=6)


if __name__=='__main__':unittest.main()
