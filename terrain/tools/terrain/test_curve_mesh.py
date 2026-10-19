"""曲面生成器の平面再現と容量制御を確認する。"""
import struct,unittest,zlib
import numpy as np
from curve_mesh import fit

class CurveTests(unittest.TestCase):
    def test_plane(self):
        y,x=np.indices((1025,1025));grid=(100+x*.2+y*.3).astype(np.float32)
        body,report,interp=fit(grid,4096)
        self.assertLess(report['maxError'],.001)
        self.assertLessEqual(len(body),4096)
        raw=zlib.decompress(body);magic,nv,nt=struct.unpack_from('<4sII',raw)
        self.assertEqual(magic,b'YCT1')
        self.assertEqual(len(raw),12+nv*20+nt*132)
        self.assertTrue(np.isfinite(np.frombuffer(raw,'<f4',nt*30,12+nv*20+nt*12)).all())

if __name__=='__main__':unittest.main()
