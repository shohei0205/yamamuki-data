"""高精度モデルの分割評価で近傍木を再利用しても結果が変わらないことを確認する。"""
import unittest,json,struct
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree
from free_mls import operator
from mls_generated_stable import DirectSupport
from mls_high_precision import evaluate
class HighPrecisionTests(unittest.TestCase):
 def data(self):
  p=np.random.default_rng(90).uniform(-10000,10000,(128,2));q=np.random.default_rng(91).uniform(-7000,7000,(40,2));return p,q
 def test_cached_operator_exactly_matches(self):
  p,q=self.data();support=DirectSupport(p);a=operator(p,q,support,curvature=1e-5);b=operator(p,q,support,curvature=1e-5,tree=support.tree);np.testing.assert_array_equal(a.indptr,b.indptr);np.testing.assert_array_equal(a.indices,b.indices);np.testing.assert_array_equal(a.data,b.data)
 def test_chunked_values_match(self):
  p,q=self.data();z=100+.01*p[:,0]+.02*p[:,1];support=DirectSupport(p);a=operator(p,q,support,curvature=1e-5)@z;np.testing.assert_allclose(evaluate(p,z,q,support,chunk=7),a,rtol=0,atol=1e-10)
 def test_saved_high_precision_header_and_validation(self):
  root=Path('build/mls-high-precision-yari-v2');info=json.loads((root/'info.json').read_text());path=root/info['selectedFolder']/'model.mlq1'
  with path.open('rb') as f:magic,n,xy,z,codec=struct.unpack('<4sIffB',f.read(17))
  self.assertEqual(magic,b'MLQ1');self.assertEqual(n,8388581);self.assertEqual(n,info['controls']);self.assertEqual(path.stat().st_size,info['bytes']);self.assertTrue(info['validationSummary']['converged']);self.assertLess(info['validationSummary']['measuredMaxError'],1)
 def test_browser(self):
  result=json.loads(Path('build/mls-generated-preview/yari-levels/browser-high-precision-verification.json').read_text());self.assertTrue(result['passed']);self.assertEqual(result['error'],0)
 def test_wrong_tree_size_rejected(self):
  p,q=self.data()
  with self.assertRaises(ValueError):operator(p,q,5000,tree=cKDTree(p[:64]))
if __name__=='__main__':unittest.main()
