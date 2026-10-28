"""支持範囲の安定化と全生成点の高速復号を確認する。"""
import unittest,json,sqlite3
from pathlib import Path
import numpy as np
from mls_generated_controls import generate
from mls_generated_stable import generate_linear,encode,decode,DirectSupport
from free_mls import operator
class StableTests(unittest.TestCase):
 def parent(self):
  xy=np.rint(np.random.default_rng(44).uniform(-10000,10000,(64,2))*4)/4;return np.column_stack((xy,100+.01*xy[:,0]+.02*xy[:,1])).astype('<f4')
 def test_same_xy_for_both_rounds(self):
  p=self.parent();saved=p.copy()
  for r in (1,2):np.testing.assert_array_equal(generate_linear(p,r)[:,:2],generate(p,r))
  np.testing.assert_array_equal(p,saved)
 def test_roundtrip_and_plane(self):
  p=self.parent();added=generate_linear(p,1);z=added[:,2].astype(float)+.5;b=encode(p,z,1,.125);d=decode(p,b);np.testing.assert_array_equal(d[:,:2],added[:,:2]);np.testing.assert_allclose(d[:,2],z,atol=.001)
 def test_direct_support_plane(self):
  p=self.parent();q=np.random.default_rng(3).uniform(-7000,7000,(50,2));a=operator(p[:,:2],q,DirectSupport(p[:,:2]),curvature=1e-5);np.testing.assert_allclose(a@p[:,2],100+.01*q[:,0]+.02*q[:,1],atol=.001)
 def test_final_validation_and_saved_parent(self):
  report=json.loads(Path('build/mls-generated-stable-v1/final-validation.json').read_text());self.assertTrue(report['passed']);self.assertEqual(report['blobBytes'],48368)
  points={}
  with sqlite3.connect('build/mls-generated-stable-v1/quality.sqlite3') as db:
   from mls_quant_codec import decode_quant
   for e,parent,codec,raw,b in db.execute('SELECT * FROM levels ORDER BY error DESC'):
    p=decode(points[parent],b) if parent else decode_quant(b)
    if parent:
     old=points[parent].copy();p=np.vstack((old,p));np.testing.assert_array_equal(old,p[:len(old)])
    points[e]=p
  self.assertEqual(len(points[25]),23685)
 def test_wrong_parent(self):
  p=self.parent();added=generate_linear(p,1);b=encode(p,added[:,2],1,1);p[0,0]+=100
  with self.assertRaises(ValueError):decode(p,b)
if __name__=='__main__':unittest.main()
