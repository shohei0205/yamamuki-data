"""ポリゴンの誤差、境界の接続、欠損と圧縮復元を確認する。"""
import unittest
import numpy as np
from polygon_dem import simplify,encode_mesh,decode_mesh


def raster(vertices,faces,size):
    result=np.full((size,size),np.nan)
    y,x=np.indices((size,size))
    for face in faces:
        a,b,c=vertices[face]
        denominator=(b[1]-c[1])*(a[0]-c[0])+(c[0]-b[0])*(a[1]-c[1])
        w=((b[1]-c[1])*(x-c[0])+(c[0]-b[0])*(y-c[1]))/denominator
        v=((c[1]-a[1])*(x-c[0])+(a[0]-c[0])*(y-c[1]))/denominator
        u=1-w-v
        inside=(w>=-1e-6)&(v>=-1e-6)&(u>=-1e-6)
        result[inside]=(w*a[2]+v*b[2]+u*c[2])[inside]
    return result


class PolygonTests(unittest.TestCase):
    def test_error_bound_at_original_samples(self):
        y,x=np.indices((17,17));height=(100*np.exp(-((x-7)**2+(y-9)**2)/18)+x*3).astype(np.float32)
        vertices,faces=simplify(height,5)
        restored=raster(vertices,faces,17)
        self.assertTrue(np.isfinite(restored).all())
        self.assertLessEqual(np.max(np.abs(restored-height)),10.001)

    def test_missing_samples_are_not_bridged(self):
        height=np.full((9,9),100.,dtype=np.float32);height[3:6,3:6]=np.nan
        vertices,faces=simplify(height,10)
        restored=raster(vertices,faces,9)
        self.assertTrue(np.isnan(restored[3:6,3:6]).all())

    def test_shared_border_is_identical(self):
        y,x=np.indices((17,17));left=(y*5+x*5).astype(np.float32)
        right=(y*5+80+50*np.sin(x/3)*np.sin(y/2)).astype(np.float32)
        a,_=simplify(left,10);b,_=simplify(right,10)
        self.assertEqual({tuple(v[1:]) for v in a if v[0]==16},{tuple(v[1:]) for v in b if v[0]==0})

    def test_implicit_mesh_roundtrip_and_flat_surface(self):
        y,x=np.indices((257,257));height=((x+y)*5).astype(np.float32)
        vertices,faces,tree=simplify(height,10,True)
        self.assertLess(len(faces),1500)
        packed=encode_mesh(vertices,faces,tree=tree)
        self.assertLess(len(packed),2048)
        decoded,triangles=decode_mesh(packed)
        self.assertTrue(np.array_equal(vertices,decoded));self.assertTrue(np.array_equal(faces,triangles))
        with self.assertRaises(Exception):decode_mesh(packed[:-10])


if __name__=='__main__':unittest.main()
