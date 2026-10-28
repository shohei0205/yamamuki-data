"""全アンカー配置の集計と、選択方式の完全復元を確認する。"""
import json,sqlite3,unittest
from pathlib import Path
import numpy as np
from compare_sibson_anchors import ROOT,ERRORS,NESTED,REGIONS
from sibson_shared_codec import load_shared
from mls_quant_codec import decode_quant

class AnchorResultsTest(unittest.TestCase):
 def test_plans_and_models(self):
  report=json.loads((ROOT/'report.json').read_text());plans=report['placements'];self.assertEqual(len(plans),64)
  self.assertEqual(len({tuple(p['anchors']) for p in plans}),64);self.assertTrue(all(800 in p['anchors'] for p in plans))
  self.assertTrue(report['allVerticesBitExact']);self.assertEqual(report['extraCompressionError'],0)
  for strategy in report['strategies']:
   if strategy['id'].startswith('depth'):
    limit=int(strategy['id'][5:]);self.assertLessEqual(strategy['maxReferences'],limit)
    self.assertEqual(strategy['blobBytes'],min(p['blobBytes'] for p in plans if p['maxReferences']<=limit))
   self.assertEqual(len(strategy['rows']),21)
   with sqlite3.connect((ROOT/(strategy['id']+'.sqlite3')).resolve().as_uri()+'?mode=ro',uri=True) as db:
    self.assertEqual(db.execute('SELECT sum(length(blob)) FROM levels').fetchone()[0],strategy['blobBytes'])
    self.assertEqual((ROOT/(strategy['id']+'.sqlite3')).stat().st_size,strategy['sqliteBytes'])
    for row in strategy['rows']:
     p,_,stats=load_shared(db,row['region'],row['error'])
     expected=decode_quant((NESTED/row['region']/f"{row['error']:g}-natural-nested{row['error']:g}"/'model.mlq1').read_bytes())
     np.testing.assert_array_equal(p,expected)
     self.assertEqual(stats['bytes'],row['requiredBytes']);self.assertEqual(stats['levels'],row['requiredLevels'])
     self.assertLessEqual(len(stats['levels']),strategy['maxReferences'])
     if row['error'] in strategy['anchors']:self.assertEqual(stats['levels'],[row['error']])

if __name__=='__main__':unittest.main()
