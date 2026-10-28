"""GPU候補木が真の自然近傍を欠落させないことを確かめる。"""
import unittest,json,struct
from pathlib import Path
import numpy as np
from scipy.spatial import cKDTree
from scipy.stats import qmc
from mls_quant_codec import decode_quant
from weighted_sibson import WeightedSibson
ROOT=Path('build/sibson-weighted-v1')
class TestGPUIndex(unittest.TestCase):
 def test_conservative_index(self):
  for region in ['fuji','yari','tsukuba']:
   for error in [25,50]:
    data=(ROOT/'gpu-controls'/f'{region}-{error}.bin').read_bytes();size=struct.unpack_from('<I',data)[0];info=json.loads(data[4:4+size]);offset=4+size;records=np.frombuffer(data,'<u4',info['pointBytes']//4,offset).reshape(-1,8);offset+=info['pointBytes']+info['cellBytes'];nodes=np.frombuffer(data,'<u4',info['nodeBytes']//4,offset).reshape(-1,8);offset+=info['nodeBytes'];refs=np.frombuffer(data,'<u4',info['referenceBytes']//4,offset);points=decode_quant((ROOT/f'{region}-{error}'/'original.mlq1').read_bytes());coordinates=records.copy().view('<f4')[:,:2]*1000;distance,ids=cKDTree(points[:,:2]).query(coordinates);self.assertLess(distance.max(),.002);self.assertEqual(len(np.unique(ids)),len(ids));self.assertEqual(info['unsafeLeaves'],0)
    query=qmc.Sobol(2,scramble=True,seed=882).random_base2(9)*8000-4000;a=WeightedSibson(points).basis(query)
    for i,q in enumerate(query):
     bounds=np.asarray(info['indexBounds']).copy();node=0
     for depth in range(24):
      entry=nodes[node]
      if entry[0]==0xffffffff:break
      middle=(bounds[:2]+bounds[2:])/2;quadrant=0
      for axis in range(2):
       if q[axis]>=middle[axis]:quadrant+=1<<axis;bounds[axis]=middle[axis]
       else:bounds[axis+2]=middle[axis]
      node=entry[quadrant]
     actual=set(a.indices[a.indptr[i]:a.indptr[i+1]][a.data[a.indptr[i]:a.indptr[i+1]]>1e-9]);candidate=set(ids[refs[entry[4]:entry[4]+entry[5]]]);self.assertTrue(actual<=candidate,(region,error,i,actual-candidate))
if __name__=='__main__':unittest.main()
