"""誤差指定から生成した自然近傍モデルを検証する。"""
import json
import unittest
from pathlib import Path
import numpy as np
from mls_quant_codec import decode_quant
from mls_lpf_dem import FilteredDem
from sibson_surface import SibsonSurface

class ErrorDrivenTest(unittest.TestCase):
 def test_results(self):
  for region in ['fuji','yari','tsukuba']:
   folder=Path('build/sibson-error-driven-v1')/region/'50-natural-error50'
   info=json.loads((folder/'info.json').read_text());points=decode_quant((folder/'model.mlq1').read_bytes())
   history=json.loads((folder/'iterations.json').read_text());trials=json.loads((folder/'lpf-trials.json').read_text())
   self.assertEqual(info['samples'],11222500);self.assertLessEqual(info['measuredMaxError'],50)
   self.assertTrue(info['converged']);self.assertEqual(info['controls'],len(points))
   self.assertEqual(len(np.unique(points[:,:2],axis=0)),len(points))
   self.assertLessEqual(info['sigmaMeters'],25);self.assertEqual(info['requestedSigmaMeters'],25)
   self.assertLessEqual(trials[-1]['filterOnlyMaxError'],25)
   self.assertLessEqual(history[-1]['measuredMaxError'],50)
   self.assertGreater(len(history),1);self.assertTrue(all(a['controls']<b['controls'] for a,b in zip(history,history[1:])))
   self.assertFalse(info['additionalDisplaySmoothing'])
   filtered=FilteredDem(region,info['sigmaMeters'])
   self.assertLessEqual(float(np.max(abs(filtered.sample(points[:,:2])-points[:,2]))),.06251)
   model=SibsonSurface(points)
   np.testing.assert_allclose(model(points[:,:2]),points[:,2],atol=1e-6)
   xy=np.random.default_rng(61).integers(1,1024,(256,2))
   for span in [8000,32000]:
    q=(xy/1024-.5)*span
    field=np.fromfile(Path('build/mls-generated-preview/dem-regions')/region/('50-natural-error50-'+str(span)+'.bin'),'<f4').reshape(1025,1025,3)
    target=np.fromfile(Path('build/mls-generated-preview/dem-regions')/region/('50-target-error50-'+str(span)+'.bin'),'<f4').reshape(1025,1025,3)
    np.testing.assert_allclose(field[xy[:,1],xy[:,0],0],model(q),atol=.00013,rtol=0)
    np.testing.assert_allclose(target[xy[:,1],xy[:,0],0],filtered.sample(q),atol=.00013,rtol=0)

if __name__=='__main__':unittest.main()
