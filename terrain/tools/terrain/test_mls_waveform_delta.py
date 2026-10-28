"""波形差分の復号と量子化誤差を確認する。"""
import unittest,json,sqlite3
from pathlib import Path
import numpy as np
from mls_waveform_delta import encode,decode,sample
class WaveTests(unittest.TestCase):
 def test_transform_error_bound(self):
  rng=np.random.default_rng(5);g=rng.normal(size=(33,33));step=.25;r=decode(encode(g,step))
  self.assertLessEqual(np.linalg.norm(g-r),np.sqrt(g.size)*step/2+1e-8)
 def test_constant_and_plane_sampling(self):
  axis=np.linspace(-16000,16000,33);x,y=np.meshgrid(axis,axis);g=100+.001*x+.002*y;q=np.array([[0,0],[1000,-2000]])
  np.testing.assert_allclose(sample(decode(encode(g,.0001)),q),100+.001*q[:,0]+.002*q[:,1],atol=.001)
 def test_invalid(self):
  with self.assertRaises(ValueError):decode(b'bad')
  with self.assertRaises(ValueError):encode(np.zeros((4,5)),1)
 def test_saved_residuals(self):
  report=json.loads(Path('build/mls-waveform-delta-v2/report.json').read_text())
  with sqlite3.connect('build/mls-waveform-delta-v2/quality.sqlite3') as db:
   for e,parent,codec,raw,b in db.execute("SELECT * FROM levels WHERE codec='WFD1'"):
    info=json.loads(raw);self.assertTrue(info['audit']['converged']);self.assertTrue(info['heldout']['converged']);self.assertEqual(len(b),info['bytes']);self.assertEqual(decode(b).shape,(info['n'],info['n']))
  self.assertLess(report['blobBytes'],report['xyzDeltaBlobBytes'])
if __name__=='__main__':unittest.main()
