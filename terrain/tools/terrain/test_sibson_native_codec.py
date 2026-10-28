import unittest
import numpy as np
from sibson_native_codec import *
class TestNativeCodec(unittest.TestCase):
 def test_integer_streams(self):
  rng=np.random.default_rng(501)
  for v in [np.array([],np.int64),np.zeros(257,np.int64),np.full(129,-53),rng.integers(-2**31,2**31,1025),np.r_[np.zeros(300),1,-1,2,-2,1000000].astype(np.int64)]:
   for rice in [False,True]:np.testing.assert_array_equal(decode_stream(encode_stream(v,rice)),v)
 def test_points(self):
  rng=np.random.default_rng(52);v=rng.integers(-2000000,2000000,(1001,3))
  for rice in [False,True]:np.testing.assert_array_equal(decode_independent(encode_independent(v,rice=rice)),v)
 def test_parent_references_and_fresh(self):
  parents=[np.array([[0.,0.,10.],[1.,1.,20.],[2.,2.,30.]],np.float32),np.array([[5.,5.,40.]],np.float32)];refs=np.array([2,0xffffffff,1<<24,0,1],np.uint32);dz=np.array([1,0,-2,0,5]);fresh=np.array([[8,16,240]])
  for rice in [False,True]:
   blob=encode_delta(refs,dz,fresh,2,rice);r,d,f=decode_delta_arrays(blob);np.testing.assert_array_equal(r,refs);np.testing.assert_array_equal(d,dz);np.testing.assert_array_equal(f,fresh);out=decode_delta(blob,parents);np.testing.assert_array_equal(out,np.array([[2,2,30.125],[.5,1,30],[5,5,39.75],[0,0,10],[1,1,20.625]]))
 def test_truncated_and_extra(self):
  blob=encode_independent(np.arange(36).reshape(-1,3))
  for end in range(len(blob)):
   with self.assertRaises((ValueError,IndexError,struct.error)):decode_independent(blob[:end])
  with self.assertRaises(ValueError):decode_independent(blob+b'\0')
if __name__=='__main__':unittest.main()
