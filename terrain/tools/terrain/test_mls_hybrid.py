"""量子化コーデックと差分XYZの検証。"""
import unittest,sqlite3,json
import numpy as np
from mls_quant_codec import encode_quant,decode_quant
class CodecTests(unittest.TestCase):
 def test_quantized_roundtrip(self):
  p=np.random.default_rng(99).uniform(-1000,1000,(100,3))
  expected=np.rint(p/[.25,.25,.125])*[.25,.25,.125]
  for codec in (0,1):
   restored=decode_quant(encode_quant(p,codec=codec))
   self.assertEqual(sorted(map(tuple,restored)),sorted(map(tuple,expected)))
 def test_empty_and_invalid(self):
  self.assertEqual(decode_quant(encode_quant(np.empty((0,3)))).shape,(0,3))
  for p in (np.array([[np.nan,0,0]]),np.array([[1e20,0,0]])):
   with self.assertRaises(ValueError):encode_quant(p)
  with self.assertRaises(ValueError):decode_quant(b'bad')
 def test_parent_xyz_unchanged(self):
  with sqlite3.connect('build/mls-fixedxyz-prune-v2/quality.sqlite3') as db:
   rows=db.execute('SELECT error,parent_error,points FROM levels ORDER BY error DESC').fetchall()
  resolved={}
  for e,parent,b in rows:
   p=decode_quant(b)
   if parent is not None:
    base=resolved[parent];p=np.vstack((base,p));np.testing.assert_array_equal(base,p[:len(base)])
   resolved[e]=p
  for coarse,fine in ((200,100),(50,25)):
   with sqlite3.connect('build/mls-quant-codec-v1/quality.sqlite3') as db:b=db.execute('SELECT points FROM levels WHERE error=?',(fine,)).fetchone()[0]
   self.assertEqual(sorted(map(tuple,resolved[fine])),sorted(map(tuple,decode_quant(b))))
 def test_selected_package(self):
  resolved={}
  with sqlite3.connect('build/mls-hybrid-selected-v2/quality.sqlite3') as db:
   for e,parent,raw,b in db.execute('SELECT error,parent_error,info,points FROM levels ORDER BY error DESC'):
    info=json.loads(raw);self.assertTrue(info['audit']['converged']);self.assertTrue(info['heldout']['converged']);self.assertTrue(info['freshValidation']['converged'])
    p=decode_quant(b)
    if parent is not None:
     old=resolved[parent];p=np.vstack((old,p));np.testing.assert_array_equal(p[:len(old)],old)
    resolved[e]=p
  self.assertEqual(set(resolved),{25,50,100,200})
  for e in (25,100):
   with sqlite3.connect('build/mls-quant-codec-v1/quality.sqlite3') as db:b=db.execute('SELECT points FROM levels WHERE error=?',(e,)).fetchone()[0]
   self.assertEqual(sorted(map(tuple,resolved[e])),sorted(map(tuple,decode_quant(b))))
if __name__=='__main__':unittest.main()
