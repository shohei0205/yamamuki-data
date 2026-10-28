"""点数連動LPFの保存データと比較用目標を検証する。"""
import json
import unittest
from pathlib import Path
import numpy as np
from mls_quant_codec import decode_quant
from mls_lpf_dem import FilteredDem
from sibson_surface import SibsonSurface

class ScaledLpfTest(unittest.TestCase):
 def test_results(self):
  for region in ['fuji','yari','tsukuba']:
   fixed_target=np.fromfile(Path('build/mls-generated-preview/dem-regions')/region/'50-target-8000.bin','<f4').reshape(1025,1025,3)
   xy=np.array([[91,351],[512,512],[823,891]]);q=(xy/1024-.5)*8000
   np.testing.assert_allclose(fixed_target[xy[:,1],xy[:,0],0],FilteredDem(region,88.38834764831843).sample(q),atol=.00013,rtol=0)
   for count in [16384,8192,4096,2048,1024,512,256,128]:
    name='50-natural-scale-'+str(count);folder=Path('build/sibson-scaled-lpf-v1')/region/name
    info=json.loads((folder/'info.json').read_text());p=decode_quant((folder/'model.mlq1').read_bytes())
    oldname='50-natural-more' if count==16384 else '50-natural-xy' if count==8192 else '50-natural-'+str(count)
    oldroot='sibson-surface-v2' if count>=8192 else 'sibson-low-count-v1'
    old=decode_quant((Path('build')/oldroot/region/oldname/'model.mlq1').read_bytes())
    np.testing.assert_array_equal(p[:,:2],old[:,:2]);self.assertEqual(len(p),count)
    self.assertAlmostEqual(info['sigmaMeters'],8000/np.sqrt(count))
    self.assertEqual(info['samples'],11222500);self.assertFalse(info['additionalDisplaySmoothing'])
    target=FilteredDem(region,info['sigmaMeters']);self.assertLessEqual(np.max(abs(target.sample(p[:,:2])-p[:,2])),.06251)
    model=SibsonSurface(p)
    for span in [8000,32000]:
     field=np.fromfile(Path('build/mls-generated-preview/dem-regions')/region/(name+'-'+str(span)+'.bin'),'<f4').reshape(1025,1025,3)
     ref=np.fromfile(Path('build/mls-generated-preview/dem-regions')/region/(info['filteredTargetKey']+'-'+str(span)+'.bin'),'<f4').reshape(1025,1025,3)
     q=(xy/1024-.5)*span
     np.testing.assert_allclose(field[xy[:,1],xy[:,0],0],model(q),atol=.00013,rtol=0)
     np.testing.assert_allclose(ref[xy[:,1],xy[:,0],0],target.sample(q),atol=.00013,rtol=0)
    if count==8192:np.testing.assert_array_equal(p,old)

if __name__=='__main__':unittest.main()
