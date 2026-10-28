"""少点数の自然近傍モデルの点数と保存・描画を確認する。"""
import json
import unittest
from pathlib import Path
import numpy as np
from mls_quant_codec import decode_quant
from sibson_surface import SibsonSurface

class LowCountTest(unittest.TestCase):
 def test_results(self):
  for region in ['fuji','yari','tsukuba']:
   for count in [4096,2048,1024,512,256,128]:
    name='50-natural-'+str(count);folder=Path('build/sibson-low-count-v1')/region/name
    info=json.loads((folder/'info.json').read_text());blob=(folder/'model.mlq1').read_bytes();p=decode_quant(blob)
    self.assertEqual(len(p),count);self.assertEqual(info['controls'],count)
    self.assertEqual(len(np.unique(p[:,:2],axis=0)),count)
    self.assertEqual(info['samples'],11222500);self.assertEqual(info['bytes'],len(blob))
    self.assertAlmostEqual(info['sigmaMeters'],88.38834764831843)
    self.assertFalse(info['additionalDisplaySmoothing'])
    for c in [[-16000,-16000],[-16000,16000],[16000,-16000],[16000,16000]]:
     self.assertTrue(np.any(np.all(p[:,:2]==c,axis=1)))
    model=SibsonSurface(p);np.testing.assert_allclose(model(p[:,:2]),p[:,2],atol=1e-7)
    for span in [8000,32000]:
     grid=np.fromfile(Path('build/mls-generated-preview/dem-regions')/region/(name+'-'+str(span)+'.bin'),'<f4').reshape(1025,1025,3)
     xy=np.random.default_rng(57).integers(1,1024,(256,2));q=(xy/1024-.5)*span
     np.testing.assert_allclose(grid[xy[:,1],xy[:,0],0],model(q),atol=.00013,rtol=0)

if __name__=='__main__':unittest.main()
