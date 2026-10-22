import unittest
import numpy as np
from compare_landcover_storage import fractions,pack_two,unpack_two,COLORS

class StorageTest(unittest.TestCase):
    def test_area_average_not_class_number_average(self):
        weights=fractions(np.array([[10,50],[10,50]],np.uint8),1)
        self.assertAlmostEqual(float(weights.sum()),1)
        np.testing.assert_allclose((weights@COLORS)[0,0],(COLORS[1]+COLORS[5])/2)

    def test_two_class_roundtrip(self):
        weights=np.zeros((3,4,12),np.float32)
        weights[:,:,1]=.6;weights[:,:,8]=.4
        restored=unpack_two(pack_two(weights))
        np.testing.assert_allclose(restored,weights@COLORS,atol=.3)

if __name__=='__main__':unittest.main()
