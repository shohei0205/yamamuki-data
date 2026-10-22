import unittest
import numpy as np
from generate_landcover_hills import denoise_plains

class PlainTextureTest(unittest.TestCase):
    def test_water_survives_plain_smoothing(self):
        rgb=np.full((65,65,3),160,dtype=np.float32)
        rgb[:,32]=[77,123,146]
        result=denoise_plains(rgb,np.zeros((65,65)))
        np.testing.assert_array_equal(result[:,32],rgb[:,32])

    def test_plain_noise_reduced_hills_unchanged(self):
        rng=np.random.default_rng(42)
        rgb=np.repeat(rng.uniform(130,190,(65,65,1)).astype(np.float32),3,axis=2)
        plain=denoise_plains(rgb,np.zeros((65,65)))
        self.assertLess(plain.std(),rgb.std()*.2)
        np.testing.assert_array_equal(denoise_plains(rgb,np.ones((65,65))),rgb)

if __name__=='__main__':unittest.main()
