"""全国補修の共有点追加と上限を検証する。"""
import sqlite3,struct,unittest,zlib
from unittest.mock import patch
import numpy as np
import national_priority_mesh as pipeline

class NationalPriorityTests(unittest.TestCase):
 def database(self,blocked=False):
  db=sqlite3.connect(':memory:');db.executescript('CREATE TABLE tiles(level,x,y,geometry,hints,mask,stats,PRIMARY KEY(level,x,y));CREATE TABLE requests(level,gx,gy,z,PRIMARY KEY(level,gx,gy));CREATE TABLE next_active(level,x,y,PRIMARY KEY(level,x,y));')
  points=[[0,0,0],[256,0,0],[0,256,0],[256,256,0]];faces=np.array([[0,1,2],[1,3,2]])
  hints=zlib.compress(struct.pack('<4sIf',b'YNH1',4,8.)+bytes(16))
  for x in [0,1]:
   s=dict(sampleGridSize=257,qualityOriginalBaseVertices=16384 if blocked and x==1 else 4,qualityAddedVertices=0)
   row=(2,x,0,b'',hints,b'', '{}');db.execute('INSERT INTO tiles VALUES(?,?,?,?,?,?,?)',(2,x,0,*pipeline.encode(row,points,faces,s)))
  db.execute('INSERT INTO requests VALUES(2,256,128,0)');db.commit();return db
 def test_shared_point_is_added_to_both_tiles(self):
  db=self.database()
  with patch.object(pipeline.national,'source_samples',lambda x,y:(np.zeros_like(x),np.ones_like(x,dtype=bool))):self.assertEqual(pipeline.apply_requests(db,2),2)
  a=pipeline.decode(db.execute('SELECT * FROM tiles WHERE x=0').fetchone())[0];b=pipeline.decode(db.execute('SELECT * FROM tiles WHERE x=1').fetchone())[0]
  self.assertIn([256,128,0],a);self.assertIn([0,128,0],b);self.assertEqual(db.execute('SELECT count(*) FROM requests').fetchone()[0],0)
 def test_budget_blocks_both_sides(self):
  db=self.database(True)
  with patch.object(pipeline.national,'source_samples',lambda x,y:(np.zeros_like(x),np.ones_like(x,dtype=bool))):self.assertEqual(pipeline.apply_requests(db,2),0)
  for row in db.execute('SELECT * FROM tiles'):self.assertEqual(len(pipeline.decode(row)[0]),4)
 def test_corner_targets(self):self.assertEqual(len(pipeline.boundary_targets(2,256,256)),4)

if __name__=='__main__':unittest.main()
