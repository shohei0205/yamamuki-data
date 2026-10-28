"""親配置の更新でも全中間点採用と点数を維持することを確認する。"""
import unittest,json
from pathlib import Path
import numpy as np
from mls_parent_child_layout import pin_corners
from mls_generated_stable import decode,generate_linear
class LayoutTests(unittest.TestCase):
 def test_corner_anchors_and_precision(self):
  p=pin_corners(np.random.default_rng(5).uniform(-15900,15900,(128,2)))
  self.assertEqual(len(p),128);self.assertEqual(np.all(abs(p)==16000,axis=1).sum(),4);np.testing.assert_array_equal(p*4,np.rint(p*4))
 def test_all_patterns_same_count_and_parent_fixed(self):
  root=Path('build/mls-parent-child-layout-v1');counts=[];positions=[]
  for i in range(3):
   folder=root/f'iteration-{i}';parent=np.fromfile(folder/'parent.f32','<f4').reshape(-1,3);saved=parent.copy();added=decode(parent,(folder/'residual-step8.mlg2').read_bytes());expected=generate_linear(parent,1);np.testing.assert_array_equal(added[:,:2],expected[:,:2]);np.testing.assert_array_equal(parent,saved);counts.append((len(parent),len(added)));positions.append(parent[:,:2].copy())
  self.assertEqual(counts,[(8192,24569)]*3);self.assertFalse(np.array_equal(positions[0],positions[2]))
 def test_common_comparison_improves_peak_and_global(self):
  rows=json.loads(Path('build/mls-parent-child-layout-v1/common-step8.json').read_text());a,b=rows[0],rows[2]
  self.assertEqual(a['controls'],b['controls']);self.assertEqual(a['step'],b['step']);self.assertLess(b['validation']['measuredMaxError'],a['validation']['measuredMaxError']);self.assertLess(b['validation']['measuredPeakMaxError'],a['validation']['measuredPeakMaxError']);self.assertFalse(b['validation']['converged'])
if __name__=='__main__':unittest.main()
