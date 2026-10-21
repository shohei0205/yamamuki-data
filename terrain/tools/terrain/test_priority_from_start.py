import unittest,sqlite3,struct,zlib
from pathlib import Path
import numpy as np
from compare_priority_from_start import measure,ROOT
class PriorityFromStartTests(unittest.TestCase):
 def test_metric_plane_and_weights(self):
  grid=np.zeros((9,9));p=np.array([[0,0,1],[8,0,1],[0,8,1],[8,8,1]],float);f=np.array([[0,1,2],[1,3,2]]);r=measure(p,f,grid,np.full((9,9),.5),np.ones((9,9),bool));self.assertEqual(r['rmse'],1);self.assertEqual(r['maximum'],1);self.assertEqual(r['weightedMaximum'],2)
 def test_equal_budget_and_shared_boundary(self):
  if not (ROOT/'build/terrain-priority-comparison.ready').exists():self.skipTest('比較生成中')
  a=sqlite3.connect((ROOT/'build/terrain-priority-staged-compare.sqlite3').as_uri()+'?mode=ro',uri=True);b=sqlite3.connect((ROOT/'build/terrain-priority-unified-compare.sqlite3').as_uri()+'?mode=ro',uri=True)
  for db in [a,b]:
   seen={};count=0
   for l,x,y,g in db.execute('select level,x,y,geometry from tiles'):
    raw=zlib.decompress(g);n=struct.unpack_from('<I',raw,8)[0];other=zlib.decompress((b if db is a else a).execute('select geometry from tiles where level=? and x=? and y=?',(l,x,y)).fetchone()[0]);self.assertEqual(n,struct.unpack_from('<I',other,8)[0]);p=np.frombuffer(raw,'<f4',count=n*3,offset=16).reshape(-1,3);width=1000*2**l
    ids=np.frombuffer(raw,'<u2',offset=16+n*12).reshape(-1,3);self.assertLess(int(ids.max()),n);u=p[ids[:,1],:2]-p[ids[:,0],:2];v=p[ids[:,2],:2]-p[ids[:,0],:2];self.assertTrue(((u[:,0]*v[:,1]-u[:,1]*v[:,0])>0).all())
    for q in p:
     if min(abs(float(q[0])),abs(float(q[1])),abs(float(q[0])-width),abs(float(q[1])-width))>.001:continue
     key=(l,round((float(q[0])+x*width)*1e5),round((float(q[1])+y*width)*1e5))
     if key in seen:self.assertEqual(seen[key],float(q[2]));count+=1
     else:seen[key]=float(q[2])
   self.assertGreater(count,100)
if __name__=='__main__':unittest.main()
