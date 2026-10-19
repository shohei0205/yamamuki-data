"""法線の縮小と固定形状の精度を確認する。"""
from io import BytesIO
import unittest

import numpy as np
from PIL import Image
from scipy.spatial import Delaunay

from fixed_mesh_preview import normal_png, mesh_blob
from irregular_dem import simplify_irregular


class FixedMeshTests(unittest.TestCase):
    def test_flat_normal(self):
        flat = np.zeros((32, 32), np.float32)
        body = normal_png((flat, flat, flat, 0, 0, 31, 31), 8)
        pixels = np.asarray(Image.open(BytesIO(body)))
        np.testing.assert_array_equal(pixels, np.broadcast_to([128, 128, 255], pixels.shape))

    def test_reduce_directions_before_normalizing(self):
        flat = np.zeros((32, 32), np.float32)
        slope = np.tile([4., -4.], (32, 16)).astype(np.float32)
        body = normal_png((flat, slope, flat, 0, 0, 32, 32), 1)
        pixels = np.asarray(Image.open(BytesIO(body)))
        np.testing.assert_array_equal(pixels[0, 0], [128, 128, 255])

    def test_finer_candidates(self):
        y,x=np.indices((65,65))
        grid=(1000*np.exp(-((x-32)**2+(y-32)**2)/120)+20*np.sin(x*.5)).astype(np.float32)
        counts=[]
        for tolerance in (20,10,5):
            body,report=mesh_blob(grid,tolerance)
            self.assertLessEqual(report['maxError'],tolerance+.01)
            self.assertEqual(report['bytes'],len(body))
            counts.append(report['triangles'])
        self.assertLess(counts[0],counts[1])
        self.assertLess(counts[1],counts[2])

    def test_peak_and_error(self):
        y, x = np.indices((33, 33))
        grid = (2000*np.exp(-((x-16)**2+(y-16)**2)/80)).astype(np.float32)
        vertices, faces, error = simplify_irregular(grid, 20)
        self.assertLess(len(faces), 32*32*2)
        self.assertEqual(vertices[:, 2].max(), grid.max())
        self.assertLessEqual(error, 20.001)
        tri = Delaunay(vertices[:, :2])
        points = np.column_stack([x.ravel(), y.ravel()])
        ids = tri.find_simplex(points)
        self.assertTrue((ids >= 0).all())
        transforms = tri.transform[ids]
        weights = np.einsum('ijk,ik->ij', transforms[:, :2], points-transforms[:, 2])
        weights = np.column_stack([weights, 1-weights.sum(axis=1)])
        restored = (vertices[tri.simplices[ids], 2]*weights).sum(axis=1)
        self.assertLessEqual(np.abs(restored-grid.ravel()).max(), 20.001)


if __name__ == '__main__':
    unittest.main()
