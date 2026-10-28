"""GPU用候補表と、C実装の参照標本を作る。"""
import json,struct,sys,unittest
from pathlib import Path
import numpy as np
from scipy.stats import qmc
from sibson_pixel import build
from sibson_surface import SibsonSurface

class PixelDataTest(unittest.TestCase):
 def test_candidate_table_contains_natural_neighbors(self):
  xy=np.vstack((qmc.Sobol(2,scramble=True,seed=48).random_base2(7)*10000-1000,[[-2000,-2000],[10000,-2000],[-2000,10000],[10000,10000]]))
  points=np.column_stack((xy,100+xy[:,0]*.1));model=SibsonSurface(points);b=build(points,[0,0,8000,8000]);size=struct.unpack_from('<I',b)[0];info=json.loads(b[4:4+size]);offset=4+size;records=np.frombuffer(b[offset:offset+info['pointBytes']],'<f4').reshape(-1,4);offset+=info['pointBytes'];cells=np.frombuffer(b[offset:offset+info['cellBytes']],'<u4').reshape(-1,2);offset+=info['cellBytes'];refs=np.frombuffer(b[offset:],'<u4')
  self.assertEqual(info['unsupportedCells'],0)
  for q in qmc.Sobol(2,scramble=True,seed=42).random_base2(6)*8000:
   ij=np.minimum((q/info['step']).astype(int),info['gridN']-1);start,count=cells[ij[1]*info['gridN']+ij[0]];candidate={tuple(v) for v in records[refs[start:start+count],:2]+4000}
   distances=np.sum((model.circles[:,:2]-q)**2,axis=1);ids=np.unique(model.tri[distances<=model.circles[:,2]*(1+1e-12)])
   for i in ids:self.assertTrue(any(np.linalg.norm(np.asarray(v)-model.xy[i])<.01 for v in candidate))

def fixtures():
 from sibson_region128_server import ROOT,database,restore
 for error in [800,100,50]:
  with database('yari') as db:points=restore(db,7,7,error)
  model=SibsonSurface(points);origin=np.array([60000,60000]);q=(qmc.Sobol(2,scramble=True,seed=17).random_base2(8)*7980-3990).astype(np.float32)
  core=points[np.all((points[:,:2]>[56500,56500])&(points[:,:2]<[63500,63500]),axis=1)]
  q=np.vstack((q,(core[:16,:2]-origin+np.array([.25,.5])).astype(np.float32)))
  h=model(q.astype(float)+origin);(ROOT/f'gpu-reference-yari{error}.json').write_text(json.dumps(dict(region='yari',x=7,y=7,error=error,queries=q.tolist(),heights=h.tolist())),encoding='utf-8')

if __name__=='__main__':
 if '--fixtures' in sys.argv:fixtures()
 else:unittest.main()
