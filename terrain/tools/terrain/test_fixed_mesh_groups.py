"""広域BLOBの頂点・接続・法線画像の独立領域を確認する。"""
from io import BytesIO
import struct
import unittest
import zlib

import numpy as np
from PIL import Image

from fixed_mesh_groups import atlas_png, group_mesh


class GroupTests(unittest.TestCase):
    def test_baked_vertices_and_indices(self):
        points = np.array([[-12000, 12000, 0], [12000, 12000, 100], [-12000, -12000, 200]], '<f4')
        indices = np.array([0, 1, 2], '<u4')
        body = zlib.compress(struct.pack('<II', 3, 3)+points.tobytes()+indices.tobytes())
        raw = zlib.decompress(group_mesh(body, 0, 0))
        nv, ni = struct.unpack_from('<II', raw)
        self.assertEqual((nv, ni), (300, 300))
        positions = np.frombuffer(raw, '<f4', nv*3, 8).reshape(-1, 3)
        faces = np.frombuffer(raw, '<u4', ni, 8+nv*12)
        uv = np.frombuffer(raw, '<f4', nv*4, 8+nv*12+ni*4).reshape(-1, 4)
        self.assertEqual(len(set(faces)), 300)
        np.testing.assert_array_equal(positions[3]-positions[0], [24000, 0, 0])
        np.testing.assert_array_equal(uv[0], [0, 0, 0, 0])
        np.testing.assert_array_equal(uv[-1, 2:], [9, 9])

    def test_atlas_padding_and_copies(self):
        pixels = np.array([[[0, 0, 255], [255, 0, 0]], [[0, 255, 0], [255, 255, 255]]], np.uint8)
        source = BytesIO()
        Image.fromarray(pixels).save(source, format='PNG')
        atlas = np.asarray(Image.open(BytesIO(atlas_png(source.getvalue()))))
        self.assertEqual(atlas.shape, (60, 60, 3))
        np.testing.assert_array_equal(atlas[2:4, 2:4], pixels)
        np.testing.assert_array_equal(atlas[56:58, 56:58], pixels)
        np.testing.assert_array_equal(atlas[0, 0], pixels[0, 0])


if __name__ == '__main__':
    unittest.main()
