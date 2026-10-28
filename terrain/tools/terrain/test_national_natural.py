import json,sqlite3,unittest
from pathlib import Path
import numpy as np
from national_natural import *
from sibson_native_codec import encode_independent,encode_delta
ROOT=Path(__file__).resolve().parents[2]
class TestNationalNatural(unittest.TestCase):
 def test_settings_and_depth(self):
  config=json.loads((ROOT/'NATIONAL_NATURAL_SETTINGS.json').read_text());levels={l['z']:l for l in config['levels']};self.assertEqual(list(levels),list(range(8,15)))
  for z in levels:
   depth=0;parent=levels[z]['parent_z']
   while parent is not None:depth+=1;parent=levels[parent]['parent_z']
   self.assertLessEqual(depth,2);self.assertEqual(depth,levels[z]['max_parent_depth'])
  self.assertEqual(levels[14]['error_m'],12.5);self.assertEqual(levels[13]['parent_z'],12)
 def test_xyz(self):
  self.assertEqual(xyz(0,0,8),(128,128));self.assertEqual(xyz(180,0,8),(255,128));self.assertEqual(xyz(-180,0,8),(0,128))
  self.assertAlmostEqual(tile_width(8),156543.03392804097);self.assertAlmostEqual(tile_width(15),1222.99245256282)
 def test_integer_rebase_and_precision(self):
  x,y=xyz(138.7,36.3,15);child=(15,x,y);parent=(14,x//2,y//2);p=origin_q(parent);o=origin_q(child)
  global_parent=np.array([[p[0]+15,p[1]+17,30000],[o[0]+7,o[1]+9,30100]],np.int64);local=global_parent.copy();local[:,:2]-=p;pb=encode_independent(local);restored_parent=decode_global_q(pb,parent);np.testing.assert_array_equal(restored_parent,global_parent)
  refs=np.array([1,0xffffffff,0],np.uint32);dz=np.array([3,0,-4]);fresh=np.array([[19,23,30200]]);blob=encode_delta(refs,dz,fresh,1);actual=decode_global_q(blob,child,[restored_parent]);expected=np.array([[o[0]+7,o[1]+9,30103],[o[0]+19,o[1]+23,30200],[p[0]+15,p[1]+17,29996]],np.int64);np.testing.assert_array_equal(actual,expected)
  gpu=global_q_to_gpu(actual[:2],child);reference=actual[:2].astype(float)*[XY,XY,Z];reference[:,:2]-=[(x+.5)*tile_width(15),(y+.5)*tile_width(15)];self.assertLess(abs(gpu-reference).max(),.0001)
  naive=(actual[:2,:2].astype(float)*XY).astype(np.float32).astype(float);self.assertGreater(abs(naive-actual[:2,:2]*XY).max(),.1);self.assertLess(point_id(int(o[0]),int(o[1])),2**63)
 def test_schema(self):
  db=sqlite3.connect(':memory:');db.executescript((ROOT/'tools/terrain/national_natural.schema.sql').read_text());db.execute('INSERT INTO generations VALUES(1,?,?,?,?)',('nt1:test',bytes(32),bytes(32),'2026-10-28T00:00:00Z'))
  db.execute('INSERT INTO levels VALUES(8,800,NULL)');db.execute('INSERT INTO levels VALUES(9,400,NULL)');blob=encode_independent(np.array([[0,0,1]]));db.execute('INSERT INTO tiles VALUES(8,128,128,1,1,1,1,1,1,?,?)',(bytes(32),blob));db.commit()
  with self.assertRaises(sqlite3.IntegrityError):db.execute('INSERT INTO tiles VALUES(8,256,128,1,1,1,1,1,1,?,?)',(bytes(32),blob))
  with self.assertRaises(sqlite3.IntegrityError):db.execute('INSERT INTO tile_parents VALUES(8,128,128,0,8,128,128)')
  self.assertEqual(db.execute('PRAGMA application_id').fetchone()[0],int.from_bytes(b'YNN1','big'))
if __name__=='__main__':unittest.main()
