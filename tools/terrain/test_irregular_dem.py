"""不規則メッシュの誤差上限、境界の接続と欠損を確認する。"""
import unittest
import numpy as np
from irregular_dem import simplify_irregular,encode_irregular,decode_irregular
from test_polygon_dem import raster


class IrregularTests(unittest.TestCase):
    def test_error_bound_on_uneven_surface(self):
        y,x=np.indices((17,17));grid=(120*np.exp(-((x-7)**2+(y-9)**2)/15)+x*2).astype(np.float32)
        vertices,faces,error=simplify_irregular(grid,8)
        restored=raster(vertices,faces,17)
        self.assertTrue(np.isfinite(restored).all())
        self.assertLessEqual(np.max(np.abs(restored-grid)),8.01)
        self.assertLessEqual(error,8.01)

    def test_neighboring_tile_borders_match(self):
        y,x=np.indices((17,17));left=(x*5+y*5+20*np.sin(y/2)).astype(np.float32)
        right=(80+y*5+20*np.sin(y/2)+60*np.sin(x/3)*np.sin(y/2)).astype(np.float32)
        a,faces,_=simplify_irregular(left,10);b,_,_=simplify_irregular(right,10)
        self.assertEqual({tuple(v[1:]) for v in a if v[0]==16},{tuple(v[1:]) for v in b if v[0]==0})
        used=set(faces.ravel().tolist())
        self.assertTrue(all(i in used for i,v in enumerate(a) if v[0] in (0,16) or v[1] in (0,16)))

    def test_missing_cells_are_not_bridged(self):
        grid=np.full((17,17),100.,np.float32);grid[6:10,6:10]=np.nan
        vertices,faces,_=simplify_irregular(grid,10)
        restored=raster(vertices,faces,17)
        self.assertTrue(np.isnan(restored[6:10,6:10]).all())

    def test_connectivity_roundtrip_and_fingerprint(self):
        from scipy.spatial import Delaunay
        import lzma
        from polygon_dem import HEADER
        vertices=np.asarray([[0,0,100],[0,256,100],[256,0,100],[256,256,100],[100,120,200]],dtype=np.float32)
        faces=Delaunay(vertices[:,:2]).simplices.astype(np.uint32)[1:]
        body=encode_irregular(vertices,faces)
        restored,triangles=decode_irregular(body)
        np.testing.assert_array_equal(vertices,restored);np.testing.assert_array_equal(faces,triangles)
        raw=bytearray(lzma.decompress(body[HEADER.size:]));raw[-1]^=1
        with self.assertRaisesRegex(ValueError,'復元が一致'):
            decode_irregular(body[:HEADER.size]+lzma.compress(raw))


if __name__=='__main__':unittest.main()
