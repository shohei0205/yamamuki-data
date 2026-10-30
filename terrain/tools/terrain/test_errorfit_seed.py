"""初期点が他品質の点数や並びに依存しないことを確認。"""
import unittest
import tempfile,json
from unittest.mock import patch
from pathlib import Path
from types import SimpleNamespace
import numpy as np
from errorfit_seed import corner_support

class CornerSupportTest(unittest.TestCase):
 def test_actual_dem_corners_not_budget(self):
  xy=np.array([[-5,-5],[5,-5],[-5,5],[5,5],[0,0]],float);z=np.array([1,2,3,4,999.])
  p=corner_support([(xy,z)],10,np.array([100.,200.]))
  np.testing.assert_array_equal(p,np.column_stack((xy[:4],z[:4])))
  # 入力順・ブロック分割・中心の候補密度は初期点数を変えない。
  q=corner_support([(xy[4:],z[4:]),(xy[:2],z[:2]),(xy[2:4],z[2:4])],10,np.array([100.,200.]))
  np.testing.assert_array_equal(p,q)
 def test_missing_corners_share_valid_point(self):
  p=corner_support([(np.array([[0.,0.],[1,1]]),np.array([12.56,np.nan]))],10,np.zeros(2))
  np.testing.assert_array_equal(p,[[0,0,12.5]])
 def test_empty_rejected(self):
  with self.assertRaises(ValueError):corner_support([],10,np.zeros(2))

class ErrorOnlyFitTest(unittest.TestCase):
 def test_greedy_addition_meets_dem_error_without_fine_budget(self):
  import errorfit_full_tile as f
  with tempfile.TemporaryDirectory() as tmp:
   root=Path(tmp);base=root/'base';src=base/'fuji/800/0-0';src.mkdir(parents=True);(base/'fuji/source').mkdir()
   (base/'fuji/source/region.json').write_text(json.dumps(dict(origin=[0,0])))
   info=dict(region='fuji',error=800,x=0,y=0,side=16,origin=[128,128],coreBounds=[120,120,136,136],worldFieldBounds=[112,112,144,144],bounds=[-16,-16,16,16],halo=8)
   (src/'packet.json').write_text(json.dumps(info))
   guards=np.array([[-1000,-1000,0],[1000,-1000,0],[-1000,1000,0],[1000,1000,0.]])
   # 参照メッシュの実点が何点あっても、そこから点数を引き継がない。
   xy=np.array([[x,y,0] for y in range(-8,9) for x in range(-8,9)],float)
   np.savez(src/'control-gradient.npz',points=np.vstack((guards,xy)))
   inherited=root/'build/natural-raw-c1-128-quarter-v1/fuji/800/0-0';inherited.mkdir(parents=True)
   np.savez(inherited/'control-gradient.npz',points=np.zeros((999,3)))
   yy,xx=np.mgrid[:256,:256];dem=100*np.exp(-((xx+.5-128)**2+(yy+.5-128)**2)/8)
   def c1(p,**kwargs):return SimpleNamespace(gradient=np.zeros((len(p),2)),fast_model=kwargs['surface'])
   with patch.multiple(f,ROOT=root,BASE=base,OUT=root/'out',STEP=1.,tile=lambda x,y:dem if (x,y)==(0,0) else np.full((256,256),np.nan),GlobalC1=c1,close_cells=lambda x:None,pack=lambda *args,**kwargs:(b'test',{'unsafeLeaves':0})):
    a=f.build('fuji',50,0,0,source_error=800,seed_policy='dem-corners')
    with patch.object(f,'OUT',root/'controls'),patch.object(f,'GlobalC1',side_effect=AssertionError('中間GPU生成は禁止')):
     controls=f.build('fuji',50,0,0,source_error=800,seed_policy='dem-corners',controls_only=True)
     self.assertTrue(controls['controlsOnly']);self.assertLessEqual(controls['max'],50)
     self.assertTrue((root/'controls/fuji/50/0-0/control-gradient.npz').exists())
   self.assertEqual(a['initialPoints'],4);self.assertGreater(a['finalPoints'],4)
   self.assertEqual(a['insertionBatch'],1);self.assertLessEqual(a['max'],50)
   self.assertTrue(a['history'][-1]['finalAudit'])
   increments=np.diff([h['points'] for h in a['history']]);self.assertTrue(np.all((increments==0)|(increments==1)))

if __name__=='__main__':unittest.main()
