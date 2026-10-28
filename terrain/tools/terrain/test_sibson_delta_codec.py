"""親曲面の可逆差分と参照破損の検出を確認する。"""
import sqlite3,unittest
import numpy as np
from mls_quant_codec import encode_quant,decode_quant
from sibson_surface import SibsonSurface
from sibson_delta_codec import encode_delta,decode_delta,load_level,checksum

class DeltaCodecTest(unittest.TestCase):
 def setUp(self):
  xy=np.array([[-16000,-16000],[-16000,16000],[16000,-16000],[16000,16000],[0,0],[2000,2000]],float)
  self.parent=decode_quant(encode_quant(np.column_stack((xy,2000+.01*xy[:,0]+.02*xy[:,1])),xy=.0625,zstep=.125))
  rng=np.random.default_rng(42);xy=np.vstack((xy,rng.uniform(-15000,15000,(50,2))))
  self.child=decode_quant(encode_quant(np.column_stack((xy,2000+.01*xy[:,0]+.02*xy[:,1]+100*np.exp(-np.sum(xy*xy,axis=1)/2e7))),xy=.0625,zstep=.125))
  self.model=SibsonSurface(self.parent)
 def database(self):
  db=sqlite3.connect(':memory:');db.execute('CREATE TABLE levels(region TEXT,error REAL,parent_error REAL,blob BLOB,sha256 BLOB,PRIMARY KEY(region,error))')
  db.execute('INSERT INTO levels VALUES(?,?,?,?,?)',('test',800,None,encode_quant(self.parent,xy=.0625),checksum(self.parent)))
  db.execute('INSERT INTO levels VALUES(?,?,?,?,?)',('test',400,800,encode_delta(self.child,self.model,800),checksum(self.child)))
  return db
 def test_exact_and_metadata(self):
  blob=encode_delta(self.child,self.model,800)
  np.testing.assert_array_equal(decode_delta(blob,self.model,800),self.child)
  with self.assertRaises(ValueError):decode_delta(blob,self.model,400)
  with self.assertRaises(ValueError):decode_delta(blob[:8],self.model,800)
  p,_,stats=load_level(self.database(),'test',400)
  np.testing.assert_array_equal(p,self.child);self.assertEqual(stats['levels'],[400,800])
 def test_damage_and_missing_parent(self):
  db=self.database();db.execute('UPDATE levels SET sha256=? WHERE error=400',(b'0'*32,))
  with self.assertRaises(ValueError):load_level(db,'test',400)
  db=self.database();db.execute('DELETE FROM levels WHERE error=800')
  with self.assertRaises(ValueError):load_level(db,'test',400)
 def test_invalid_dependency_and_unquantized_height(self):
  db=self.database();db.execute('UPDATE levels SET parent_error=400 WHERE error=400')
  with self.assertRaises(ValueError):load_level(db,'test',400)
  p=self.child.astype(float);p[0,2]+=.01
  with self.assertRaises(ValueError):encode_delta(p,self.model,800)

if __name__=='__main__':unittest.main()
