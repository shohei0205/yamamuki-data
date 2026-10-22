import unittest
import numpy as np
from retile_shared_mesh import repair_serialized_faces


class RetileTests(unittest.TestCase):
    def test_rejects_diagonal_that_would_add_area(self):
        p=np.array([[0,0,0],[4,0,0],[2,-.001,0],[4,-4,0]],float)
        f=np.array([[0,1,2],[1,2,3]],np.uint32)
        with self.assertRaises(ValueError):repair_serialized_faces(p,f)

    def test_valid_faces_keep_original_connections(self):
        p=np.array([[0,0,0],[128000,0,1],[128000,128000,2],[0,128000,3]],float)
        f=np.array([[0,1,2],[0,2,3]],np.uint32)
        np.testing.assert_array_equal(repair_serialized_faces(p,f),f)


if __name__=='__main__':unittest.main()
