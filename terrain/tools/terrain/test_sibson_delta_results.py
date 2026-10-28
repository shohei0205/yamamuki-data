"""全モデルの差分復元・容量集計・親参照を検証する。"""
import json,sqlite3,unittest
from pathlib import Path
import numpy as np
from compare_sibson_delta import source,ERRORS,ANCHORS
from mls_quant_codec import decode_quant
from sibson_delta_codec import load_level

class DeltaResultsTest(unittest.TestCase):
 def test_all_models(self):
  root=Path('build/sibson-delta-comparison-v1');report=json.loads((root/'report.json').read_text())
  self.assertEqual(len(report['rows']),21);self.assertTrue(report['allVerticesBitExact']);self.assertEqual(report['extraHeightError'],0)
  for mode in ['independent','chain','anchors']:
   with sqlite3.connect((root/(mode+'.sqlite3')).resolve().as_uri()+'?mode=ro',uri=True) as db:
    self.assertEqual(db.execute('SELECT count(*) FROM levels').fetchone()[0],21)
    self.assertEqual(db.execute('SELECT sum(length(blob)) FROM levels').fetchone()[0],report['totals'][mode]['blobBytes'])
    self.assertEqual((root/(mode+'.sqlite3')).stat().st_size,report['totals'][mode]['sqliteBytes'])
    for region in ['fuji','yari','tsukuba']:
     for error in ERRORS:
      expected=decode_quant((source(region,error)/'model.mlq1').read_bytes());p,_,stats=load_level(db,region,error)
      np.testing.assert_array_equal(p,expected)
      row=next(r for r in report['rows'] if r['region']==region and r['error']==error)
      self.assertEqual(stats['bytes'],row['timings'][mode]['requiredBytes'])
      if mode=='independent' or mode=='anchors' and error in ANCHORS:self.assertEqual(stats['levels'],[error])
      elif mode=='anchors':self.assertEqual(len(stats['levels']),2)
      else:self.assertEqual(len(stats['levels']),ERRORS.index(error)+1)

if __name__=='__main__':unittest.main()
