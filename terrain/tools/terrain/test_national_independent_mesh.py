"""全国独立LODの定義・座標変換・生成結果を確認する。"""
import json,struct,unittest,zlib
import numpy as np
from national_independent_mesh import *
class NationalTest(unittest.TestCase):
 def test_definitions_projection(self):
  specs=definitions();self.assertEqual([s['widthMeters'] for s in specs],[1000*2**i for i in range(8)])
  self.assertEqual([s['maxErrorMeters'] for s in specs],[2,5,10,25,50,100,200,400])
  lon=np.array([122.,138.,154.]);lat=np.array([24.,36.,46.]);x,y=project(lon,lat);a,b=inverse(x,y);np.testing.assert_allclose(a,lon,atol=1e-10);np.testing.assert_allclose(b,lat,atol=1e-10)
 def test_generation_and_shared_edge(self):
  initialize(str(ROOT/'build/terrain-lod.sqlite3'),str(ROOT/'build/terrain-rust-target/release/terrain-adaptive-tin.exe'))
  x,y=project(138.7274,35.3606);tx,ty=int(x//1000),int(y//1000);results=[]
  for job in [(0,tx,ty),(0,tx+1,ty),(3,int(x//8000),int(y//8000)),(7,int(x//128000),int(y//128000))]:
   _,result=generate_tile(job);self.assertIsNotNone(result);geometry,hints,mask,meta=result;stats=json.loads(meta);raw=zlib.decompress(geometry);magic,n,m,nv,ni=struct.unpack_from('<4sHHII',raw)
   self.assertEqual(magic,b'YNT1');self.assertEqual(n,m);self.assertTrue(128<=nv<=1024);self.assertEqual(len(raw),16+nv*8+ni*2);self.assertEqual(len(zlib.decompress(hints)),12+nv*4);self.assertEqual(len(zlib.decompress(mask)),(n*n+7)//8);self.assertTrue(np.isfinite(stats['curveFitSampleRmseMeters']));results.append(stats)
  self.assertEqual(results[0]['boundaryDigests'][1],results[1]['boundaryDigests'][0])
if __name__=='__main__':unittest.main()
