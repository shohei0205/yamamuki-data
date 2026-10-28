"""富士山2048点の局所近似と山頂配分を検証する。"""
import json,sqlite3,unittest,zlib
import numpy as np
from free_mls import LocalSupport,operator,reference_values,SPAN
from scipy.stats import qmc,norm
class PeakTests(unittest.TestCase):
    def read(self,path):
        with sqlite3.connect(path) as db:raw,b=db.execute("SELECT info,points FROM surfaces WHERE region='fuji' AND budget=2048").fetchone()
        d=np.frombuffer(zlib.decompress(b),'<f4');return json.loads(raw),d[1:].reshape(-1,3),b
    def test_budget_distribution_and_unmodified_local_positions(self):
        old,original,_=self.read('build/free-mls-v3/mls.sqlite3');local,points,_=self.read('build/free-mls-fuji-local-v1/mls.sqlite3');peak,focused,b=self.read('build/free-mls-fuji-peak20-v1/mls.sqlite3')
        np.testing.assert_array_equal(points[:,:2],original[:,:2]);self.assertEqual(len(focused),2048);self.assertEqual(len(b),peak['storedBytes']);self.assertGreater(peak['within500'],local['within500']*10);self.assertLess(peak['peakAreaRmse'],local['peakAreaRmse']*.3)
        near=focused[np.linalg.norm(focused[:,:2],axis=1)<=500,:2]
        for x,y in [(1,1),(1,-1),(-1,1),(-1,-1)]:self.assertGreater(((near[:,0]*x>0)&(near[:,1]*y>0)).sum(),10)
    def test_restored_peak_metrics_and_coverage(self):
        info,p,b=self.read('build/free-mls-fuji-peak20-v1/mls.sqlite3');support=LocalSupport(p[:,:2],pre_smooth=False,floor_factor=.5)
        q=np.clip(norm.ppf(qmc.Sobol(2,scramble=True,seed=881).random_base2(12)),-5,5)*1000
        with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:blob=db.execute("SELECT coefficients FROM surfaces WHERE region='fuji' AND n=256").fetchone()[0]
        original=np.frombuffer(zlib.decompress(blob),'<f4').reshape(261,261);error=operator(p[:,:2],q,support,curvature=1e-5)@p[:,2]-reference_values(original,q)
        self.assertAlmostEqual(float(np.sqrt(np.mean(error**2))),info['peakAreaRmse'],places=5)
        q=(qmc.Sobol(2,scramble=True,seed=932).random_base2(12)-.5)*SPAN
        self.assertTrue(np.isfinite(operator(p[:,:2],q,support,curvature=1e-5)@p[:,2]).all())
if __name__=='__main__':unittest.main()
