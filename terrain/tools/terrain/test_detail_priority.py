import unittest
import numpy as np
from detail_priority import priority_gain,shape_priority_gain


class DetailPriorityTest(unittest.TestCase):
    def test_flat_is_suppressed_even_with_landform_weight(self):
        grid=np.zeros((65,65),np.float32)
        gain=priority_gain(grid,np.ones_like(grid),np.ones_like(grid,dtype=bool))
        self.assertTrue(np.allclose(gain,.03))

    def test_peak_retains_priority_despite_zero_center_slope(self):
        y,x=np.mgrid[-64:65,-64:65]*31.25
        grid=800*np.exp(-(x*x+y*y)/(2*500**2))
        gain=priority_gain(grid,np.ones_like(grid),np.ones_like(grid,dtype=bool))
        self.assertGreater(gain[64,64],1.)
        self.assertLess(gain[0,0],.05)

    def test_gentle_plain_has_lower_score_than_mountain_at_equal_error(self):
        y,x=np.mgrid[:129,:129]*31.25
        valid=np.ones_like(x,dtype=bool)
        plain=priority_gain(x*.01,np.zeros_like(x),valid)
        mountain=priority_gain(x*.2,np.zeros_like(x),valid)
        self.assertLess(plain[64,64],mountain[64,64]/20)
        valid[64,64]=False
        self.assertAlmostEqual(float(priority_gain(x*.2,np.ones_like(x),valid)[64,64]),.02)

class ShapePriorityTest(unittest.TestCase):
    def test_lower_mountain_gets_more_weight_for_same_error(self):
        y,x=np.mgrid[-96:97,-96:97]*31.25
        shape=np.maximum(0.,1.-np.hypot(x,y)/1500.)
        land=np.ones_like(shape);valid=np.ones_like(shape,dtype=bool)
        small=shape_priority_gain(400*shape,land,valid)
        high=shape_priority_gain(1200*shape,land,valid)
        self.assertGreater(small[96,96],high[96,96]*1.8)

    def test_flat_ground_still_cannot_spend_pool_on_height_error_alone(self):
        z=np.zeros((65,65),np.float32)
        gain=shape_priority_gain(z,z,np.ones_like(z,dtype=bool))
        self.assertLess(float(gain.max()),.1)

if __name__=='__main__':unittest.main()
