"""粗い法線の平均化と階層画像の大きさを確認する。"""
from io import BytesIO
import unittest
import numpy as np
from PIL import Image
from fixed_mesh_hierarchy import reduced_normal
from fixed_mesh_groups import atlas_png

class NormalLODTests(unittest.TestCase):
    def test_vector_average(self):
        pixels = np.array([[[255,128,255],[0,128,255]],[[255,128,255],[0,128,255]]], dtype='uint8')
        body = BytesIO()
        Image.fromarray(pixels).save(body, format='PNG')
        result = np.asarray(Image.open(BytesIO(reduced_normal(body.getvalue(), 1))))
        self.assertLessEqual(abs(int(result[0,0,0])-128), 1)
        self.assertGreaterEqual(int(result[0,0,2]), 254)

    def test_density_halves_with_each_parent(self):
        body = BytesIO()
        Image.new('RGB',(256,256),(128,128,255)).save(body,format='PNG')
        for depth in range(1,8):
            resolution = 256//2**depth
            normal = reduced_normal(body.getvalue(),resolution)
            image = Image.open(BytesIO(normal))
            self.assertEqual(image.size,(resolution,resolution))
            atlas = Image.open(BytesIO(atlas_png(normal,2,3)))
            self.assertEqual(atlas.size,((resolution+4)*2,(resolution+4)*3))

if __name__ == '__main__':
    unittest.main()
