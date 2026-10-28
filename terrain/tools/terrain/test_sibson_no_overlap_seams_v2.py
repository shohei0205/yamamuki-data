"""余白なし試験の保存座標・形状監査を確認する。"""
import json
import sqlite3
import unittest
from pathlib import Path
import numpy as np
from mls_quant_codec import decode_quant

class NoOverlapTest(unittest.TestCase):
 def test_all_tiles_have_no_halo(self):
  root=Path('build/sibson-no-overlap-seams-v2')
  self.assertEqual(json.loads((root/'progress.json').read_text())['stage'],'complete')
  for region in ['fuji','yari','tsukuba']:
   with sqlite3.connect((root/region/'tiles.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as db:
    rows=db.execute('SELECT blob,info FROM tiles').fetchall();self.assertEqual(len(rows),35)
    for blob,info in rows:
     m=json.loads(info);p=decode_quant(blob);bounds=m['bounds']
     self.assertEqual(m['fieldBounds'],bounds);self.assertEqual(m['haloPoints'],0)
     self.assertEqual(m['maxOutsideDistanceMeters'],0)
     self.assertTrue(np.all(p[:,:2]>=bounds[:2]));self.assertTrue(np.all(p[:,:2]<=bounds[2:]))
     self.assertGreater(m['audit']['boundarySamples'],3000);self.assertLessEqual(m['audit']['maxError'],m['error']);self.assertGreater(m['audit']['samples'],600000)

if __name__=='__main__':unittest.main()
