import unittest,struct,json
import numpy as np
from sibson_gpu_dem import STEP,center,raster,sample,packet
class TestRawDEM(unittest.TestCase):
 def test_native_pixels(self):
  for region in ['fuji','yari','tsukuba']:
   pixel=np.floor(center(region)/STEP).astype(int);offset=np.arange(-20,21);x=pixel[0]+offset;y=pixel[1]+offset
   q=np.c_[(x+.5)*STEP-center(region)[0]+64000,(y+.5)*STEP-center(region)[1]+64000];expected=np.array([raster(px//256,py//256)[py%256,px%256] for px,py in zip(x,y)]);np.testing.assert_allclose(sample(region,q),expected,atol=1e-5,equal_nan=True)
 def test_packet(self):
  body=packet('yari',12.5,32,32,32);size=struct.unpack_from('<I',body)[0];info=json.loads(body[4:4+size]);values=np.frombuffer(body,dtype='<f4',offset=4+size).reshape(-1,8);self.assertTrue(info['noLPF']);self.assertEqual(len(values),33**2);self.assertTrue(np.isfinite(values).all());np.testing.assert_array_equal(values[:,:4],values[:,4:])
if __name__=='__main__':unittest.main()
