"""距離計算、ゼロ面の接続、保存テクスチャの可逆復元を検証する。"""
import unittest,json,struct,zlib
from pathlib import Path
import numpy as np
from PIL import Image
from terrain_sdf import triangle_distance,extract,OUT
class SdfTests(unittest.TestCase):
 def test_triangle_distance(self):
  a=np.zeros((3,3));b=np.tile([1.,0,0],(3,1));c=np.tile([0.,1,0],(3,1));q=np.array([[.2,.3,2],[2,0,0],[-1,-1,0.]])
  np.testing.assert_allclose(triangle_distance(q,a,b,c),[2,1,np.sqrt(2)])
 def test_plane_extract(self):
  axis=np.linspace(-1,1,5);v=np.broadcast_to(axis[:,None,None],(5,5,5)).copy();p,f=extract(v,axis,axis);np.testing.assert_allclose(p[:,2],0,atol=1e-8)
  self.assertTrue(((f[:,0]!=f[:,1])&(f[:,1]!=f[:,2])&(f[:,0]!=f[:,2])).all())
  a=p[f];area=np.linalg.norm(np.cross(a[:,1]-a[:,0],a[:,2]-a[:,0]),axis=1).sum()/2;self.assertAlmostEqual(area,4,places=7)
 def test_saved_png_matches_compressed_distance(self):
  for n in [64,128,256]:
   b=(OUT/f'yari-{n}.sdf').read_bytes();size=struct.unpack('<I',b[:4])[0];meta=json.loads(b[4:4+size]);shape=meta['shape'];raw=np.frombuffer(zlib.decompress(b[4+size:]),'u1').reshape(2,-1).T.copy().view('<i2').reshape(shape)
   atlas=np.asarray(Image.open(OUT/f'yari-{n}.png'),dtype=np.int32);columns=int(np.ceil(np.sqrt(shape[0])))
   for k in range(shape[0]):np.testing.assert_array_equal(atlas[(k//columns)*n:(k//columns+1)*n,(k%columns)*n:(k%columns+1)*n]-32768,raw[k])
if __name__=='__main__':unittest.main()
