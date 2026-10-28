"""誤差から決まるMLS品質レベルと停止条件を検証する。"""
import json,sqlite3,unittest,zlib
import numpy as np
from scipy.stats import qmc
from free_mls import LocalSupport,operator,reference_values,SPAN
from mls_error_quality import requirements,next_count,peak_samples
class QualityTests(unittest.TestCase):
    def test_limits_and_peak_rule(self):
        q=np.array([[0,0],[5000,0]],float)
        self.assertTrue(requirements([-50,100],q,100)['converged'])
        self.assertFalse(requirements([51,0],q,100)['converged'])
        self.assertFalse(requirements([0,-101],q,100)['converged'])
        m=requirements([100,200],q,100);self.assertGreater(next_count(300,1000,m,100),300);self.assertEqual(next_count(990,1000,m,100),1000);self.assertEqual(next_count(1000,1000,m,100),1000)
    def test_unmet_at_point_cap_is_not_success(self):
        with sqlite3.connect('build/mls-error-quality-cap-test-v1/quality.sqlite3') as db:raw=db.execute('SELECT info FROM levels').fetchone()[0]
        info=json.loads(raw);self.assertEqual(info['controls'],256);self.assertFalse(info['converged']);self.assertEqual(info['stopReason'],'maximum-points')
    def test_saved_levels_meet_audited_requirements(self):
        axis=np.linspace(-SPAN/2,SPAN/2,257);x,y=np.meshgrid(axis,axis);q=np.vstack((np.column_stack((x.ravel(),y.ravel())),(qmc.Sobol(2,scramble=True,seed=904).random_base2(14)-.5)*SPAN,peak_samples(12,935,1000)))
        with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:b=db.execute("SELECT coefficients FROM surfaces WHERE region='fuji' AND n=256").fetchone()[0]
        original=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261);truth=reference_values(original,q)
        with sqlite3.connect('build/mls-error-quality-v7/quality.sqlite3') as db:rows=db.execute('SELECT target_error,info,points FROM levels ORDER BY target_error DESC').fetchall()
        self.assertEqual(len(rows),4);previous=0
        for target,raw,b in rows:
            info=json.loads(raw);d=np.frombuffer(zlib.decompress(b),'<f4');p=d[1:].reshape(-1,3);self.assertEqual(len(p),info['controls']);self.assertGreaterEqual(len(p),previous);previous=len(p);self.assertLessEqual(len(p),info['maximumPoints']);self.assertEqual(len(b),info['storedBytes'])
            support=LocalSupport(p[:,:2],scale=float(d[0]),pre_smooth=False,floor_factor=info['supportFloorFactor'],grid_n=info['supportGridN'])
            error=operator(p[:,:2],q,support,curvature=info['curvaturePenalty'])@p[:,2]-truth;measurement=requirements(error,q,target)
            self.assertTrue(measurement['converged']);self.assertTrue(info['converged']);self.assertAlmostEqual(measurement['measuredMaxError'],info['measuredMaxError'],places=5);self.assertAlmostEqual(measurement['measuredPeakMaxError'],info['measuredPeakMaxError'],places=5)
if __name__=='__main__':unittest.main()
