"""メッシュ法線の差分マップを検証する。"""
from io import BytesIO
import struct,unittest,zlib
import numpy as np
from PIL import Image
from fixed_normal_codec import correction_png

class CorrectionTests(unittest.TestCase):
    def mesh(self):
        vertices=np.array([[-1,-1,0],[1,-1,0],[-1,1,0],[1,1,0]],'<f4')
        indices=np.array([0,1,2,1,3,2],'<u4')
        return zlib.compress(struct.pack('<II',4,6)+vertices.tobytes()+indices.tobytes())
    def image(self,color):
        out=BytesIO();Image.new('RGB',(4,4),color).save(out,format='PNG');return out.getvalue()
    def test_reconstruct_reference(self):
        reference=np.array([160,100,240],dtype=np.float64)/127.5-1
        reference/=np.linalg.norm(reference)
        delta=np.asarray(Image.open(BytesIO(correction_png(self.image((160,100,240)),self.mesh(),2,4))),dtype=np.float64)
        restored=(delta[0,0]-128)/63.5+np.array([0,0,1])
        restored/=np.linalg.norm(restored)
        self.assertLess(np.degrees(np.arccos(np.clip(restored@reference,-1,1))),1)
    def test_atlas_padding(self):
        from fixed_mesh_groups import atlas_png
        source=atlas_png(self.image((128,128,255)),2,3)
        result=Image.open(BytesIO(correction_png(source,self.mesh(),2,4,2,3)))
        self.assertEqual(result.size,(16,24))

if __name__=='__main__':unittest.main()
