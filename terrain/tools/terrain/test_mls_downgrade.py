"""最高品位からの削減で包含関係と品質条件を維持することを確認する。"""
import json,sqlite3,unittest,zlib
from pathlib import Path
import numpy as np
from scipy.stats import qmc
from free_mls import LocalSupport,operator,reference_values,SPAN
from mls_error_quality import peak_samples,requirements
from mls_downgrade import choose_removals
class DowngradeTests(unittest.TestCase):
    def test_removal_batch_preserves_protected_points(self):
        p=np.array([[-16000,-16000],[-16000,16000],[16000,-16000],[16000,16000],[0,0],[4000,0],[8000,0]],float);protected=np.array([True]*4+[False]*3)
        removed=choose_removals(p,np.arange(len(p)),np.full(len(p),100.),protected,2);self.assertEqual(len(removed),2);self.assertTrue(np.all(removed>=4))
    def test_nested_positions_and_seed_identity(self):
        with sqlite3.connect('build/mls-error-quality-v7/quality.sqlite3') as db:seed_blob=db.execute("SELECT points FROM levels WHERE region='fuji' AND target_error=25").fetchone()[0]
        seed=np.frombuffer(zlib.decompress(seed_blob),'<f4')[1:].reshape(-1,3)
        with sqlite3.connect('build/mls-downgrade-v2/quality.sqlite3') as db:rows=db.execute('SELECT target_error,info,points FROM levels ORDER BY target_error').fetchall()
        self.assertEqual(len(rows),4);parent=set(range(len(seed)))
        for target,raw,b in rows:
            info=json.loads(raw);data=np.frombuffer(zlib.decompress(b),'<f4')[1:].reshape(-1,3);ids=np.load(f'build/mls-downgrade-v2/lineage-fuji-E{target:g}.npy',allow_pickle=False)
            self.assertTrue(set(ids).issubset(parent));parent=set(ids);np.testing.assert_array_equal(data[:,:2],seed[ids,:2]);self.assertEqual(len(ids),info['controls']);self.assertEqual(len(b),info['storedBytes']);self.assertTrue(info['converged'])
            self.assertEqual(np.all(abs(data[:,:2])==SPAN/2,axis=1).sum(),4)
            if target==25:self.assertEqual(b,seed_blob)
    def test_audited_errors_are_reproduced(self):
        axis=np.linspace(-SPAN/2,SPAN/2,257);x,y=np.meshgrid(axis,axis);q=np.vstack((np.column_stack((x.ravel(),y.ravel())),(qmc.Sobol(2,scramble=True,seed=904).random_base2(14)-.5)*SPAN,peak_samples(12,935,1000)))
        with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:b=db.execute("SELECT coefficients FROM surfaces WHERE region='fuji' AND n=256").fetchone()[0]
        original=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261);truth=reference_values(original,q)
        with sqlite3.connect('build/mls-downgrade-v2/quality.sqlite3') as db:rows=db.execute('SELECT target_error,info,points FROM levels').fetchall()
        for target,raw,b in rows:
            info=json.loads(raw);data=np.frombuffer(zlib.decompress(b),'<f4');p=data[1:].reshape(-1,3);support=LocalSupport(p[:,:2],scale=float(data[0]),pre_smooth=False,floor_factor=.5)
            error=operator(p[:,:2],q,support,curvature=1e-5)@p[:,2]-truth;m=requirements(error,q,target)
            self.assertTrue(m['converged']);self.assertAlmostEqual(m['measuredMaxError'],info['measuredMaxError'],places=5);self.assertAlmostEqual(m['measuredPeakMaxError'],info['measuredPeakMaxError'],places=5)
if __name__=='__main__':unittest.main()
