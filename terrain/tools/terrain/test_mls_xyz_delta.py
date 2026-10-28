"""MLS差分の可逆性と親XYZ固定を確認する。"""
import sqlite3,unittest
import numpy as np
from mls_xyz_delta import encode,decode
class DeltaTests(unittest.TestCase):
    def test_roundtrip(self):
        for n in (0,1,100):
            p=np.random.default_rng(4).normal(size=(n,3)).astype('<f4')
            np.testing.assert_array_equal(p,decode(encode(p)))
    def test_parent_chain(self):
        points=np.empty((0,3),'<f4');previous=None
        with sqlite3.connect('build/mls-xyz-delta-v1/delta.sqlite3') as db:
            for error,parent,blob in db.execute('SELECT error,parent_error,added FROM levels ORDER BY error DESC'):
                self.assertEqual(parent,previous);old=points.copy();points=np.vstack((points,decode(blob)))
                np.testing.assert_array_equal(points[:len(old)],old);previous=error
        self.assertEqual(len(points),32729)
if __name__=='__main__':unittest.main()
