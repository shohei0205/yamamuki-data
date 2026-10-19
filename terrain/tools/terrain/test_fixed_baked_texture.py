"""固定陰影と谷の模様の生成を確認する。"""
from io import BytesIO
import unittest,zlib
import numpy as np
from PIL import Image
from fixed_normal_codec import baked_texture, baked_mesh_correction

class BakedTests(unittest.TestCase):
    def normal(self):
        out=BytesIO();Image.new('RGB',(32,32),(128,128,255)).save(out,format='PNG');return out.getvalue()
    def test_flat_lighting(self):
        grid=np.zeros((33,33),'<f4')
        body=baked_texture(self.normal(),zlib.compress(grid.tobytes()),33,16,'baked',315,90)
        self.assertGreaterEqual(np.asarray(Image.open(BytesIO(body))).min(),254)
    def test_valley_is_darker(self):
        y,x=np.indices((65,65));grid=(-200*np.exp(-((x-32)**2+(y-32)**2)/40)).astype('<f4')
        body=baked_texture(self.normal(),zlib.compress(grid.tobytes()),65,65,'valley')
        pixels=np.asarray(Image.open(BytesIO(body)))
        self.assertLess(int(pixels[32,32]),int(pixels[0,0]))
    def test_baked_correction_reconstructs_flat_light(self):
        import struct
        points=np.array([[-1,-1,0],[1,-1,0],[-1,1,0],[1,1,0]],'<f4')
        indices=np.array([0,1,2,1,3,2],'<u4')
        mesh=zlib.compress(struct.pack('<II',4,6)+points.tobytes()+indices.tobytes())
        source=BytesIO();Image.new('L',(4,4),128).save(source,format='PNG')
        result=baked_mesh_correction(source.getvalue(),mesh,2,4,315,40)
        values=np.asarray(Image.open(BytesIO(result)),dtype=np.float64)
        restored=(.3+.7*np.sin(np.radians(40)))*2**((values-128)/63.5)
        self.assertLess(np.abs(restored-128/255).max(),.005)
        self.assertEqual(Image.open(BytesIO(result)).mode,'L')

    def test_atlas(self):
        grid=np.zeros((33,33),'<f4')
        body=baked_texture(self.normal(),zlib.compress(grid.tobytes()),33,8,'baked',315,40,2,3)
        self.assertEqual(Image.open(BytesIO(body)).size,(24,36))
        self.assertEqual(Image.open(BytesIO(body)).mode,'L')
        self.assertEqual(body[25],0)

if __name__=='__main__':unittest.main()
