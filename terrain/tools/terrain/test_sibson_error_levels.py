"""12.5～800 mの誤差指定自然近傍の保存結果を検証する。"""
import json,unittest
from pathlib import Path
import numpy as np
from mls_quant_codec import decode_quant
from sibson_surface import SibsonSurface

class ErrorLevelsTest(unittest.TestCase):
 def test_results(self):
  for region in ['fuji','yari','tsukuba']:
   for error in [12.5,25,50,100,200,400,800]:
    root='sibson-error-fine-v1' if error<50 else 'sibson-error-driven-v1' if error==50 else 'sibson-error-levels-v1'
    name=f'{error}-natural-error{error}';folder=Path('build')/root/region/name
    info=json.loads((folder/'info.json').read_text());blob=(folder/'model.mlq1').read_bytes();p=decode_quant(blob)
    history=json.loads((folder/'iterations.json').read_text());trials=json.loads((folder/'lpf-trials.json').read_text())
    self.assertEqual(info['samples'],11222500);self.assertEqual(info['targetError'],error)
    self.assertEqual(info['peakErrorLimit'],error);self.assertTrue(info['converged'])
    self.assertLessEqual(info['measuredMaxError'],error);self.assertLessEqual(history[-1]['measuredMaxError'],error)
    self.assertEqual(info['controls'],len(p));self.assertEqual(info['bytes'],len(blob));self.assertEqual(len(np.unique(p[:,:2],axis=0)),len(p))
    self.assertEqual(info['requestedSigmaMeters'],error/2);self.assertEqual(info['filterErrorBudget'],error/2)
    self.assertLessEqual(info['sigmaMeters'],error/2);self.assertEqual(info['sigmaMeters'],trials[-1]['sigmaMeters'])
    self.assertLessEqual(trials[-1]['filterOnlyMaxError'],error/2)
    self.assertFalse(info['additionalDisplaySmoothing']);self.assertEqual(info['filteredTargetKey'],f'{error}-target-error{error}')
    model=SibsonSurface(p);np.testing.assert_allclose(model(p[:,:2]),p[:,2],atol=1e-6)
    xy=np.random.default_rng(29).integers(1,1024,(256,2))
    for span in [8000,32000]:
     field=np.fromfile(Path('build/mls-generated-preview/dem-regions')/region/(name+'-'+str(span)+'.bin'),'<f4').reshape(1025,1025,3)
     np.testing.assert_allclose(field[xy[:,1],xy[:,0],0],model((xy/1024-.5)*span),atol=.00013,rtol=0)
     ref=Path('build/mls-generated-preview/dem-regions')/region/(info['filteredTargetKey']+'-'+str(span)+'.bin');self.assertEqual(ref.stat().st_size,field.nbytes)

if __name__=='__main__':unittest.main()
