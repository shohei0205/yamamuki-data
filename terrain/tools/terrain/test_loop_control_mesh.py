import unittest,sqlite3,zlib,struct
from pathlib import Path
import numpy as np
ROOT=Path(__file__).resolve().parents[2]
class ControlMeshTests(unittest.TestCase):
 def test_coordinates_topology_and_shared_vertices(self):
  path=ROOT/'build/terrain-loop-control.sqlite3'
  if not path.exists():self.skipTest('比較データが未生成')
  db=sqlite3.connect(path.as_uri()+'?mode=ro',uri=True);src=sqlite3.connect((ROOT/'build/terrain-regional-priority-levels-v5.sqlite3').as_uri()+'?mode=ro',uri=True);seen={};shared=0;changed=0
  for l,x,y,g in db.execute('select level,x,y,geometry from tiles'):
   raw=zlib.decompress(g);old=zlib.decompress(src.execute('select geometry from tiles where level=? and x=? and y=?',(l,x,y)).fetchone()[0]);self.assertEqual(raw[:16],old[:16]);n=struct.unpack_from('<I',raw,8)[0];p=np.frombuffer(raw,dtype='<f4',count=n*3,offset=16).reshape(-1,3);q=np.frombuffer(old,dtype='<f4',count=n*3,offset=16).reshape(-1,3);np.testing.assert_array_equal(p[:,:2],q[:,:2]);self.assertEqual(raw[16+n*12:],old[16+n*12:]);changed+=int((p[:,2]!=q[:,2]).sum())
   for point in p:
    key=(l,round((float(point[0])+x*1000*2**l)*1e5),round((float(point[1])+y*1000*2**l)*1e5))
    if key in seen:self.assertEqual(seen[key],float(point[2]));shared+=1
    else:seen[key]=float(point[2])
  self.assertGreater(shared,100);self.assertGreater(changed,100)
if __name__=='__main__':unittest.main()
