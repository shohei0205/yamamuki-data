"""独立12.5mモデルの単独復号と比較条件を検証する。"""
import json,unittest
from pathlib import Path
import numpy as np
from mls_quant_codec import decode_quant
from mls_generated_stable import generate_linear
class IndependentTests(unittest.TestCase):
 def test_standalone_and_same_positions(self):
  root=Path('build/mls-independent-yari-E12p5-v1');p=decode_quant((root/'model.mlq1').read_bytes());metadata=json.loads((root/'metadata.json').read_text());self.assertIsNone(metadata['parent']);self.assertEqual(len(p),131033)
  parent=np.fromfile('build/mls-residual-curve-yari-v2/parent.f32','<f4').reshape(-1,3);xy=np.vstack((parent[:,:2],generate_linear(parent,1)[:,:2]));a=np.lexsort((p[:,1],p[:,0]));b=np.lexsort((xy[:,1],xy[:,0]));np.testing.assert_array_equal(p[a,:2],xy[b])
 def test_display_status_matches_final_validation(self):
  root=Path('build/mls-independent-yari-E12p5-v1');info=json.loads((root/'info.json').read_text());display=json.loads(Path('build/mls-generated-preview/yari-levels/level-12.5-independent.json').read_text());self.assertEqual(display['reconstruction'],'independent-MLS');self.assertEqual(display['bytes'],(root/'model.mlq1').stat().st_size);self.assertEqual(display['converged'],info['commonValidation']['converged']);self.assertEqual(display['measuredMaxError'],info['commonValidation']['measuredMaxError'])
 def test_browser(self):
  info=json.loads(Path('build/mls-generated-preview/yari-levels/browser-independent-verification.json').read_text());self.assertTrue(info['passed']);self.assertEqual(info['error'],0)
if __name__=='__main__':unittest.main()
