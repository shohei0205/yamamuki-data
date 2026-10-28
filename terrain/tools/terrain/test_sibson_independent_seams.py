"""任意矩形の外周と、独立生成の保存結果を確認する。"""
import json
import sqlite3
import unittest
from pathlib import Path
import numpy as np
from sibson_surface import SibsonSurface
from mls_quant_codec import decode_quant

class IndependentTilesTest(unittest.TestCase):
 def test_rectangular_hull(self):
  xy=np.array([[-10000,-2000],[2000,-2000],[2000,10000],[-10000,10000],[0,0]],float)
  model=SibsonSurface(np.column_stack((xy,100+.1*xy[:,0]-.2*xy[:,1])))
  q=np.array([[-10000,1000],[2000,4000],[0,-2000],[0,10000],[-500,200]],float)
  np.testing.assert_allclose(model(q),100+.1*q[:,0]-.2*q[:,1],atol=1e-7)

 def test_generated_audits(self):
  root=Path('build/sibson-independent-seams-v1')
  self.assertEqual(json.loads((root/'progress.json').read_text())['stage'],'complete')
  for region in ['fuji','yari','tsukuba']:
   with sqlite3.connect((root/region/'tiles.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as db:
    rows=db.execute('SELECT blob,info FROM tiles').fetchall();self.assertEqual(len(rows),35)
    for blob,info in rows:
     m=json.loads(info);p=decode_quant(blob)
     self.assertTrue(m['independentlyFitted']);self.assertIsNone(m['parentError'])
     self.assertEqual(len(p),m['points']);self.assertLessEqual(m['audit']['maxError'],m['error'])
     self.assertGreater(m['audit']['samples'],1000000)

if __name__=='__main__':unittest.main()
