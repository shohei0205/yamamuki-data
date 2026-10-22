import unittest,zlib
import numpy as np
from compare_shared_allocation import encode_surface_mask

class MeshSurfacePolicyTest(unittest.TestCase):
    def test_missing_elevation_does_not_cut_surface(self):
        valid=np.zeros((9,9),bool);valid[2:7,2:7]=True
        body=encode_surface_mask(valid)
        restored=np.unpackbits(np.frombuffer(zlib.decompress(body),np.uint8),bitorder='little')[:81]
        self.assertTrue(restored.all())
        np.testing.assert_array_equal(valid[2:7,2:7],True)
        self.assertFalse(valid[0,0])

if __name__=='__main__':unittest.main()
