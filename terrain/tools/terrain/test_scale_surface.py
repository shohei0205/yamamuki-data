"""三角形区画を使わない自由曲面と、解像度の偏りを検証する。"""
import json
import sqlite3
import unittest
import zlib
from pathlib import Path
import numpy as np
from scipy.ndimage import gaussian_filter
import scale_surface as s

class ScaleTests(unittest.TestCase):
    def reference(self):
        step=250.;origin=-20000.;axis=np.arange(161)*step+origin;x,y=np.meshgrid(axis,axis)
        return x,y,origin,step

    def test_plane_and_gradient(self):
        x,y,origin,step=self.reference();ref=1000+.03*x-.02*y;spacing=s.SPAN/np.sqrt(128)
        centers=s.centers_for_budget(128,ref,origin,step,spacing);patches=s.fit_patches(ref,origin,step,centers,spacing*1.6)
        rng=np.random.default_rng(672);q=rng.uniform(-16000,16000,(400,2));h,g=s.evaluate(patches,q,True)
        np.testing.assert_allclose(h,1000+.03*q[:,0]-.02*q[:,1],atol=.002)
        np.testing.assert_allclose(g,np.tile([.03,-.02],(len(q),1)),atol=2e-6)
        for axis in range(2):
            d=np.eye(2)[axis]*.1;fd=(s.evaluate(patches,q+d)-s.evaluate(patches,q-d))/.2
            np.testing.assert_allclose(g[:,axis],fd,atol=1e-6)

    def test_equal_mountains_do_not_get_different_resolution(self):
        x,y,origin,step=self.reference();ref=1000*np.exp(-((x-8000)**2+y*y)/(2*1600**2))+1000*np.exp(-((x+8000)**2+y*y)/(2*1600**2))
        spacing=s.SPAN/np.sqrt(512);target=gaussian_filter(ref,spacing*.3/step)
        centers=s.centers_for_budget(512,target,origin,step,spacing);patches=s.fit_patches(target,origin,step,centers,spacing*1.6)
        h=s.evaluate(patches,[[-8000,0],[8000,0]])
        self.assertLess(abs(h[0]-h[1])/np.mean(h),.08)
        self.assertTrue(np.all(h<1000));self.assertTrue(np.all(h>700))

    def test_saved_models(self):
        report=[]
        with sqlite3.connect('build/adaptive-free-surface-v7/adaptive.sqlite3') as db:rows=db.execute('select region,budget,info,model from surfaces').fetchall()
        self.assertEqual(len(rows),12)
        rng=np.random.default_rng(126);q=np.vstack((rng.uniform(-16000,16000,(300,2)),[[-16000,-16000],[16000,-16000],[16000,16000],[-16000,16000]]))
        for region,count,raw,blob in rows:
            info=json.loads(raw);patches=np.frombuffer(zlib.decompress(blob),'<f4').reshape(-1,9)
            self.assertEqual(len(patches),count);self.assertEqual(len(np.unique(patches[:,:2],axis=0)),count)
            np.testing.assert_allclose(patches[:,2],s.SPAN/np.sqrt(count)*1.6,rtol=1e-6)
            self.assertGreater(info['medianSeparation']/info['spacingMeters'],.6)
            self.assertLess(info['densityP99']/info['densityP50'],1.5)
            h,g=s.evaluate(patches,q,True);self.assertTrue(np.all(np.isfinite(h)));self.assertTrue(np.all(np.isfinite(g)))
            lookup,indices=s.bins(patches);self.assertLessEqual(lookup[:,1].max(),1024)
            # 保存する中心は元の標高格子にも描画格子にも揃えない。
            frac=abs(patches[:,:2]/62.5-np.round(patches[:,:2]/62.5));self.assertGreater(np.mean(frac>.001),.95)
            report.append(dict(region=region,budget=count,maxBin=int(lookup[:,1].max()),medianSeparation=info['medianSeparation'],densityRatio=info['densityP99']/info['densityP50']))
        Path('build/adaptive-free-surface-v7/numeric-verification.json').write_text(json.dumps(dict(passed=True,models=report),indent=2),encoding='utf-8')

if __name__=='__main__':unittest.main()
