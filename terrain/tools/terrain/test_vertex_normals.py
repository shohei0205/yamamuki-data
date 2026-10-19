"""頂点法線の2バイト符号化とメッシュ対応を検証する。"""
import unittest,sqlite3,zlib,struct,hashlib
import numpy as np
from vertex_normals import encode_oct,decode_oct

class VertexNormalTests(unittest.TestCase):
    def test_all_directions(self):
        rng=np.random.default_rng(7);n=rng.normal(size=(20000,3));n/=np.linalg.norm(n,axis=1,keepdims=True)
        packed=encode_oct(n);decoded=decode_oct(packed)
        error=np.degrees(np.arccos(np.clip((n*decoded).sum(axis=1),-1,1)))
        self.assertEqual(packed.shape,(20000,2));self.assertLess(error.max(),1)
    def test_database_binding(self):
        with sqlite3.connect('build/fixed-mesh-preview.sqlite3') as db:
            for region,tolerance,body in db.execute('SELECT region,variant,data FROM blobs WHERE kind="vertex_normal"'):
                raw=zlib.decompress(body);magic,nv=struct.unpack_from('<4sI',raw)
                mesh=zlib.decompress(db.execute('SELECT data FROM blobs WHERE region=? AND kind="mesh" AND variant=?',(region,tolerance)).fetchone()[0])
                self.assertEqual(magic,b'YVN1');self.assertEqual(len(raw),40+nv*2)
                self.assertEqual(raw[8:40],hashlib.sha256(mesh).digest())
                self.assertEqual(nv,struct.unpack_from('<I',mesh)[0])

    def test_slope_topology_and_normal_binding(self):
        with sqlite3.connect('build/fixed-mesh-preview.sqlite3') as db:
            for region,tolerance,body in db.execute('SELECT region,variant,data FROM slope_blobs'):
                raw=zlib.decompress(body);nv,ni=struct.unpack_from('<II',raw)
                old=zlib.decompress(db.execute('SELECT data FROM blobs WHERE region=? AND kind="mesh" AND variant=?',(region,tolerance)).fetchone()[0])
                self.assertEqual(raw[:8+nv*12],old[:8+nv*12])
                faces=np.frombuffer(raw,'<u4',ni,8+nv*12).reshape(-1,3)
                self.assertLess(faces.max(),nv)
                edges=np.sort(np.concatenate([faces[:,[0,1]],faces[:,[1,2]],faces[:,[2,0]]]),axis=1)
                _,counts=np.unique(edges,axis=0,return_counts=True)
                self.assertLessEqual(counts.max(),2)
                packet=zlib.decompress(db.execute('SELECT data FROM blobs WHERE region=? AND kind="vertex_normal_slope" AND variant=?',(region,tolerance)).fetchone()[0])
                self.assertEqual(packet[8:40],hashlib.sha256(raw).digest())

if __name__=='__main__':unittest.main()
