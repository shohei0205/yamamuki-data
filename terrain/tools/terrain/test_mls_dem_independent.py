"""元DEM基準の生成結果と全画素検証の整合性を確かめる。"""
import json
import unittest
from pathlib import Path
import numpy as np
from mls_quant_codec import decode_quant

class DemModelTests(unittest.TestCase):
 def test_round_trip_and_reference(self):
  root=Path('build/mls-dem-independent-yari-v1')
  info=json.loads((root/'info.json').read_text())
  points=decode_quant((root/'model.mlq1').read_bytes())
  self.assertEqual(info['reference'],'original-PNG-DEM')
  self.assertEqual(info['controls'],len(points))
  self.assertEqual(info['bytes'],(root/'model.mlq1').stat().st_size)
  self.assertTrue(np.isfinite(points).all())
 def test_native_validation_is_not_training_error(self):
  info=json.loads(Path('build/mls-dem-independent-yari-v1/info.json').read_text())
  measured=info['nativeDemValidation']
  self.assertEqual(measured['samples'],11222500)
  self.assertEqual(measured['missing'],0)
  worst=measured['worst']
  self.assertAlmostEqual(abs(worst['dem']-worst['model']),measured['measuredMaxError'])
  self.assertEqual(measured['converged'],measured['measuredMaxError']<=12.5 and measured['measuredPeakMaxError']<=6.25)

if __name__=='__main__':unittest.main()
