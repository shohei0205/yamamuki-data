"""親曲面＋残差曲面の符号化と親の保持を検証する。"""
import unittest,json,hashlib
from pathlib import Path
import numpy as np
from mls_residual_curve import encode,decode
from mls_generated_stable import generate_linear,DirectSupport
from free_mls import operator
class ResidualTests(unittest.TestCase):
 def parent(self):
  xy=np.rint(np.random.default_rng(40).uniform(-10000,10000,(64,2))*4)/4;return np.column_stack((xy,100+.01*xy[:,0]+.02*xy[:,1])).astype('<f4')
 def test_zero_residual_keeps_parent_surface_exactly(self):
  parent=self.parent();saved=parent.copy();added=generate_linear(parent,1);xy,c=decode(parent,encode(parent,np.zeros(len(added)),1));np.testing.assert_array_equal(xy,added[:,:2]);q=np.random.default_rng(8).uniform(-6000,6000,(40,2));p=operator(parent[:,:2],q,DirectSupport(parent[:,:2]),curvature=1e-5)@parent[:,2];r=operator(xy,q,DirectSupport(xy),curvature=1e-5)@c;np.testing.assert_array_equal(p+r,p);np.testing.assert_array_equal(parent,saved)
 def test_coefficients_roundtrip(self):
  p=self.parent();n=len(generate_linear(p,1));c=np.random.default_rng(8).normal(size=n);_,r=decode(p,encode(p,c,.25));self.assertLessEqual(float(abs(c-r).max()),.125+1e-8)
 def test_saved_parent_and_common_comparison(self):
  root=Path('build/mls-residual-curve-yari-v2');parent=np.fromfile(root/'parent.f32','<f4').reshape(-1,3);metadata=json.loads((root/'metadata.json').read_text());self.assertEqual(hashlib.sha256(parent.tobytes()).hexdigest(),metadata['parentSha256'])
  rows=json.loads((root/'common-validation.json').read_text());old=next(v for v in rows if v['mode']=='XYZ-inheritance-12.5');new=next(v for v in rows if v['mode']=='residual-v2');self.assertLess(new['measurement']['measuredMaxError'],old['measurement']['measuredMaxError']);self.assertFalse(new['measurement']['converged'])
 def test_browser_and_zero_fallback_baseline(self):
  info=json.loads(Path('build/mls-generated-preview/yari-levels/browser-residual-verification.json').read_text());self.assertTrue(info['passed']);self.assertEqual(info['info']['reconstruction'],'parent-plus-residual-MLS');self.assertEqual(info['error'],0)
 def test_wrong_parent_rejected(self):
  p=self.parent();n=len(generate_linear(p,1));b=encode(p,np.zeros(n),1);p[0,0]+=100
  with self.assertRaises(ValueError):decode(p,b)
if __name__=='__main__':unittest.main()
