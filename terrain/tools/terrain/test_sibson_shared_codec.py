"""共有 XY と共通点 Z 補正の可逆性を確認する。"""
import unittest,sqlite3
from unittest.mock import patch
from sibson_delta_codec import checksum
from sibson_surface import SibsonSurface
import numpy as np
from mls_quant_codec import encode_quant,decode_quant
from sibson_shared_codec import encode_shared,decode_shared,canonical,xy_keys,load_shared

class SharedCodecTest(unittest.TestCase):
 def setUp(self):
  self.parent=decode_quant(encode_quant(np.array([[-16000,-16000,20],[-16000,16000,30],[16000,-16000,40],[16000,16000,50],[0,0,200]],float),xy=.0625))
 def test_z_correction_and_added_points(self):
  child=self.parent.copy();child[:,2]+=np.arange(len(child))*.125
  child=canonical(np.vstack((child,[[123,456,100],[2000,-3000,300]])))
  blob=encode_shared(child,self.parent,800)
  np.testing.assert_array_equal(decode_shared(blob,self.parent,800),child)
  with self.assertRaises(ValueError):decode_shared(blob,self.parent,400)
 def test_no_added_and_unchanged_z(self):
  blob=encode_shared(self.parent,self.parent,800)
  np.testing.assert_array_equal(decode_shared(blob,self.parent,800),self.parent)
 def test_missing_parent_point_and_quantization(self):
  with self.assertRaises(ValueError):encode_shared(self.parent[:-1],self.parent,800)
  child=np.vstack((self.parent,[[123,456,100.01]]))
  with self.assertRaises(ValueError):encode_shared(child,self.parent,800)
  with self.assertRaises(ValueError):decode_shared(b'NSP1',self.parent,800)

 def test_only_final_surface_and_integrity(self):
  child=canonical(np.vstack((self.parent,[[123,456,100]])))
  fine=canonical(np.vstack((child,[[321,765,120]])))
  db=sqlite3.connect(':memory:');db.execute('CREATE TABLE levels(region TEXT,error REAL,parent_error REAL,blob BLOB,sha256 BLOB)')
  db.executemany('INSERT INTO levels VALUES(?,?,?,?,?)',[
   ('test',800,None,encode_quant(self.parent,xy=.0625),checksum(self.parent)),
   ('test',400,800,encode_shared(child,self.parent,800),checksum(child)),
   ('test',200,400,encode_shared(fine,child,400),checksum(fine))])
  with patch('sibson_shared_codec.SibsonSurface',wraps=SibsonSurface) as builder:
   p,_,stats=load_shared(db,'test',200)
   np.testing.assert_array_equal(p,fine);self.assertEqual(builder.call_count,1);self.assertEqual(stats['levels'],[200,400,800])
  db.execute('UPDATE levels SET sha256=? WHERE error=200',(b'0'*32,))
  with self.assertRaises(ValueError):load_shared(db,'test',200)
  db.execute('DELETE FROM levels WHERE error=800')
  with self.assertRaises(ValueError):load_shared(db,'test',200)

if __name__=='__main__':unittest.main()
