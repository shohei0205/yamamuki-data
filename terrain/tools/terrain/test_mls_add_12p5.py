"""12.5m目標版の親継承と全中間点採用を確認する。"""
import unittest,json
from pathlib import Path
import numpy as np
from mls_generated_stable import decode,generate_linear
class LevelTests(unittest.TestCase):
 def test_parent_and_all_generated_positions(self):
  out=Path('build/mls-parent-child-layout-E12p5-v1');parent=np.fromfile(out/'parent.f32','<f4').reshape(-1,3);original=parent.copy();child=decode(parent,(out/'residual.mlg2').read_bytes());expected=generate_linear(parent,1)
  self.assertEqual(len(parent),32761);self.assertEqual(len(child),98272);np.testing.assert_array_equal(child[:,:2],expected[:,:2]);np.testing.assert_array_equal(parent,original)
 def test_failed_quality_is_explicit(self):
  info=json.loads(Path('build/mls-generated-preview/yari-levels/level-12.5.json').read_text());self.assertEqual(info['targetError'],12.5);self.assertFalse(info['converged']);self.assertGreater(info['measuredMaxError'],12.5)
 def test_browser_switch(self):
  info=json.loads(Path('build/mls-generated-preview/yari-levels/browser-verification.json').read_text());self.assertTrue(info['passed']);self.assertEqual(info['error'],0)
if __name__=='__main__':unittest.main()
