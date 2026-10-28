"""LPFを固定した誤差配分、半径変更と信頼度を検証する。"""
import json
import sqlite3
import unittest
import zlib
from pathlib import Path
import numpy as np
from scipy.ndimage import gaussian_filter
from scipy.spatial import cKDTree
import scale_surface as s
import adaptive_allocation_surface as a

class AllocationTests(unittest.TestCase):
    def test_density_sampling_and_plane_with_variable_radius(self):
        density=np.ones((64,64));density[:,:32]=4
        c=a.sample_density(density,2048,726)
        fraction=np.mean(c[:,0]<0);self.assertGreater(fraction,.77);self.assertLess(fraction,.83)
        axis=np.linspace(-24000,24000,193);x,y=np.meshgrid(axis,axis);reference=500+.01*x-.02*y
        cover_axis=np.linspace(-16000,16000,257);cx,cy=np.meshgrid(cover_axis,cover_axis);cover=np.column_stack((cx.ravel(),cy.ravel()))
        c=a.distribute(density,512,s.SPAN/np.sqrt(512));r,floor=a.radii_for_centers(c,s.SPAN/np.sqrt(512),cover)
        p=a.fit_sampled(reference,-24000,250,c,r,floor)
        rng=np.random.default_rng(84);q=rng.uniform(-16000,16000,(1000,2));h,g=s.evaluate(p,q,True)
        np.testing.assert_allclose(h,500+.01*q[:,0]-.02*q[:,1],atol=.002)
        np.testing.assert_allclose(g,np.tile([.01,-.02],(len(q),1)),atol=2e-6)
        for k in range(2):
            d=np.eye(2)[k]*.1;expected=(s.evaluate(p,q+d)-s.evaluate(p,q-d))/.2
            np.testing.assert_allclose(g[:,k],expected,atol=1e-6)

    def test_real_models_improve_without_changing_lpf_or_budget(self):
        rows=[]
        with sqlite3.connect('build/adaptive-free-surface-v8/adaptive.sqlite3') as db:models=db.execute('select region,budget,info,model from surfaces').fetchall()
        self.assertEqual(len(models),12)
        rng=np.random.default_rng(835);query=np.vstack((rng.uniform(-16000,16000,(300,2)),[[-16000,-16000],[16000,-16000],[16000,16000],[-16000,16000]]))
        for region,count,raw,blob in models:
            info=json.loads(raw);p=np.frombuffer(zlib.decompress(blob),'<f4').reshape(-1,10)
            self.assertEqual(len(p),count);self.assertEqual(len(np.unique(p[:,:2],axis=0)),count)
            self.assertAlmostEqual(info['sigmaMeters'],s.SPAN/np.sqrt(count)*.3)
            self.assertTrue(np.all(p[:,2]>=info['radiusFloor']*(1-1e-6)))
            self.assertTrue(np.all((p[:,9]>0)&(p[:,9]<=1)))
            self.assertLessEqual(info['validationRmse'],info['initialValidationRmse']+1e-10)
            values,grad=s.evaluate(p,query,True);self.assertTrue(np.all(np.isfinite(values)));self.assertTrue(np.all(np.isfinite(grad)))
            lookup,indices=s.bins(p);self.assertLessEqual(lookup[:,1].max(),1024)
            if region in ['fuji','tsukuba']:
                self.assertLess(info['validationRmse']/info['initialValidationRmse'],.8)
                self.assertGreater(info['radiusMax']/info['radiusMin'],2)
            rows.append(dict(region=region,budget=count,improvement=1-info['validationRmse']/info['initialValidationRmse'],maxBin=int(lookup[:,1].max())))
        # 島状の高密度化を目的にせず、山頂周辺へ必要な配分が増えたことを確認する。
        for region in ['fuji','tsukuba']:
            counts=[]
            for version,width in [('v7',9),('v8',10)]:
                with sqlite3.connect(f'build/adaptive-free-surface-{version}/adaptive.sqlite3') as db:b=db.execute('select model from surfaces where region=? and budget=512',(region,)).fetchone()[0]
                p=np.frombuffer(zlib.decompress(b),'<f4').reshape(-1,width);counts.append(np.count_nonzero(np.linalg.norm(p[:,:2],axis=1)<6000))
            self.assertGreater(counts[1],counts[0]*1.4)
        Path('build/adaptive-free-surface-v8/numeric-verification.json').write_text(json.dumps(dict(passed=True,models=rows),indent=2),encoding='utf-8')

if __name__=='__main__':unittest.main()
