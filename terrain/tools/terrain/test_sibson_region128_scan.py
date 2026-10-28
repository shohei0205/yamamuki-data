"""局所再検査の結果を、独立した全標本評価と比較する。"""
import unittest
import numpy as np
from scipy.stats import qmc
from raw_dem_reference import STEP
from sibson_region128_scan import ScanCache
from sibson_surface import SibsonSurface
from mls_quant_codec import encode_quant,decode_quant

def height(q):return 1000+100*np.sin(q[:,0]/150)*np.cos(q[:,1]/200)+50*np.exp(-np.sum((q-1200)**2,axis=1)/10000)

class Raw:
 left=top=0
 def __init__(self):
  self.local=(np.arange(300)+.5)*STEP;x,y=np.meshgrid(self.local,self.local);self.q=np.column_stack((x.ravel(),y.ravel()));self.grid=height(self.q).reshape(300,300).astype(np.float32)
 def axes(self):return np.arange(300),np.arange(300),self.local,self.local

class CacheTest(unittest.TestCase):
 def test_added_points_equal_full_scan(self):
  raw=Raw();xy=np.vstack((qmc.Sobol(2,scramble=True,seed=8).random_base2(8)*3000,[[0,0],[3000,0],[0,3000],[3000,3000]]));cache=ScanCache(raw,block=32)
  for added in [None,[1200.125,1199.875],[2500.25,1900.5],[100,200]]:
   if added is not None:xy=np.vstack((xy,added))
   points=decode_quant(encode_quant(np.column_stack((xy,height(xy))),xy=.0625,zstep=.125));model=SibsonSurface(points)
   dirty=cache.changed(model,points)
   if added is not None:self.assertLess(len(dirty),len(cache.maximum))
   maximum,rms,_,count=cache.evaluate(model,points,25)
   reference=model(raw.q)-raw.grid.ravel()
   self.assertAlmostEqual(maximum,float(abs(reference).max()),places=7);self.assertAlmostEqual(rms,float(np.sqrt(np.mean(reference**2))),places=7);self.assertEqual(count,90000)

if __name__=='__main__':unittest.main()
