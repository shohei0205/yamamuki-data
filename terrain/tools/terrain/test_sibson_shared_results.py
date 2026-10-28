"""親XY保持の品質と、共有形式の完全復元・容量を確認する。"""
import json,sqlite3,unittest
from pathlib import Path
import numpy as np
from compare_sibson_shared import NESTED,ERRORS,ROOT
from mls_quant_codec import decode_quant
from sibson_shared_codec import load_shared,xy_keys
from sibson_delta_codec import load_level

class SharedResultsTest(unittest.TestCase):
 def test_results(self):
  report=json.loads((ROOT/'report.json').read_text());self.assertEqual(len(report['rows']),21)
  self.assertTrue(report['allSharedVerticesBitExact']);self.assertEqual(report['extraCompressionError'],0)
  for mode in ['original','nested','shared']:
   with sqlite3.connect((ROOT/(mode+'.sqlite3')).resolve().as_uri()+'?mode=ro',uri=True) as db:
    self.assertEqual(db.execute('SELECT count(*) FROM levels').fetchone()[0],21)
    self.assertEqual(db.execute('SELECT sum(length(blob)) FROM levels').fetchone()[0],report['totals'][mode]['blobBytes'])
    self.assertEqual((ROOT/(mode+'.sqlite3')).stat().st_size,report['totals'][mode]['sqliteBytes'])
    for region in ['fuji','yari','tsukuba']:
     parent=None
     for error in ERRORS:
      folder=NESTED/region/f'{error:g}-natural-nested{error:g}'
      info=json.loads((folder/'info.json').read_text());p=decode_quant((folder/'model.mlq1').read_bytes())
      self.assertLessEqual(info['measuredMaxError'],error);self.assertTrue(info['converged']);self.assertEqual(info['samples'],11222500)
      if parent is not None:self.assertTrue(np.isin(xy_keys(parent),xy_keys(p)).all())
      row=next(r for r in report['rows'] if r['region']==region and r['error']==error)
      restored,model,stats=load_shared(db,region,error) if mode=='shared' else load_level(db,region,error)
      self.assertEqual(stats['bytes'],row['timings'][mode]['requiredBytes'])
      if mode!='original':np.testing.assert_array_equal(restored,p)
      self.assertEqual(len(p),row['nestedControls']);self.assertEqual(len(p)-(len(parent) if parent is not None else 0),row['addedControls'])
      if mode=='shared':self.assertEqual(len(stats['levels']),ERRORS.index(error)+1)
      parent=p

if __name__=='__main__':unittest.main()
