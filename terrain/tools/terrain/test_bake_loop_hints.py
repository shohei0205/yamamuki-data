"""Loopの線形作用素と、保存した補助情報の整合性を検証する。"""
import hashlib,sqlite3,struct,unittest,zlib
from pathlib import Path
import numpy as np
from bake_loop_hints import operator

class LoopBakeTest(unittest.TestCase):
 def test_affine_and_boundary(self):
  points=np.array([[0,0],[2,0],[2,2],[0,2],[1,.7]],float)
  faces=np.array([[0,1,4],[1,2,4],[2,3,4],[3,0,4]])
  a,f,boundary=operator(5,faces)
  self.assertEqual(boundary,{0,1,2,3})
  for _ in range(2):b,f,_=operator(a.shape[0],f);a=b@a
  np.testing.assert_allclose(a.sum(axis=1),1,atol=1e-14)
  xy=a@points; z=a@(points[:,0]*.3-points[:,1]*.2+20)
  np.testing.assert_allclose(z,xy[:,0]*.3-xy[:,1]*.2+20,atol=1e-12)
  np.testing.assert_allclose(xy[:4],points[:4],atol=1e-14)
  u=xy[f[:,1]]-xy[f[:,0]];v=xy[f[:,2]]-xy[f[:,0]];cross=u[:,0]*v[:,1]-u[:,1]*v[:,0]
  self.assertTrue((cross>0).all())
 def test_saved_hints(self):
  root=Path(__file__).resolve().parents[2]; path=root/'build/terrain-loop-baked.sqlite3'
  if not path.exists():self.skipTest('比較用補助データは未生成')
  src=sqlite3.connect((root/'build/terrain-regional-priority-levels-v5.sqlite3').as_uri()+'?mode=ro',uri=True)
  db=sqlite3.connect(path.as_uri()+'?mode=ro',uri=True);shared={};duplicates=0
  for l,x,y,data in db.execute('select level,x,y,data from tiles'):
   raw=zlib.decompress(data);magic,n,scale=struct.unpack_from('<4sIf',raw);self.assertEqual(magic,b'YLB1');self.assertEqual(len(raw),44+3*n)
   g=zlib.decompress(src.execute('select geometry from tiles where level=? and x=? and y=?',(l,x,y)).fetchone()[0]);self.assertEqual(hashlib.sha256(g).digest(),raw[12:44])
   p=np.frombuffer(g,dtype='<f4',count=n*3,offset=16).reshape(-1,3);delta=np.frombuffer(raw,dtype='<i2',count=n,offset=44);retain=np.frombuffer(raw,dtype='u1',count=n,offset=44+n*2)
   for i,v in enumerate(p):
    key=(l,round((float(v[0])+x*1000*2**l)*1e5),round((float(v[1])+y*1000*2**l)*1e5));hint=(int(delta[i]),int(retain[i]))
    if key in shared:self.assertEqual(shared[key],hint);duplicates+=1
    else:shared[key]=hint
  self.assertGreater(duplicates,100)

if __name__=='__main__':unittest.main()
