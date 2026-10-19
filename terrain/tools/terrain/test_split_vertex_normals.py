"""既存BLOBから座標と法線が欠落なく分離されたか確認する。"""
import sqlite3,struct,unittest,zlib
from split_vertex_normals import ROOT
class SplitTest(unittest.TestCase):
 def test_roundtrip(self):
  with sqlite3.connect(ROOT/'build/progressive-split.sqlite3') as split:
   for filename,reference in [('progressive-tin-compare.sqlite3',False),('progressive-tin-reference.sqlite3',True)]:
    with sqlite3.connect(ROOT/'build'/filename) as src:
     for region,fmt,level,kind,data in src.execute('SELECT * FROM blobs'):
      if reference and fmt!='independent':continue
      target='reference' if reference else fmt
      body=zlib.decompress(split.execute('SELECT data FROM blobs WHERE region=? AND format=? AND level=? AND kind=?',(region,target,level,kind)).fetchone()[0])
      original=zlib.decompress(data)
      if kind!='vertices':self.assertEqual(body,original);continue
      normals=zlib.decompress(split.execute('SELECT data FROM blobs WHERE region=? AND format=? AND level=? AND kind="normals"',(region,target,level)).fetchone()[0]);first,n=struct.unpack_from('<II',body,4)
      reconstructed=struct.pack('<4sII',b'PVX1',first,n)+b''.join(body[12+i*8:20+i*8]+normals[12+i*2:14+i*2] for i in range(n))
      self.assertEqual(reconstructed,original)
   with sqlite3.connect(ROOT/'build/tin-optimized-compare.sqlite3') as src:
    for region,variant,level,data in src.execute('SELECT * FROM meshes'):
     args=(region,'opt_'+variant,level)
     body=zlib.decompress(split.execute('SELECT data FROM blobs WHERE region=? AND format=? AND level=? AND kind="mesh"',args).fetchone()[0]);normals=zlib.decompress(split.execute('SELECT data FROM blobs WHERE region=? AND format=? AND level=? AND kind="normals"',args).fetchone()[0]);self.assertEqual(b'YOP1'+body[4:]+normals[12:],zlib.decompress(data))
if __name__=='__main__':unittest.main()
