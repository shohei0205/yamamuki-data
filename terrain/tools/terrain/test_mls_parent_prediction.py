"""自由配置の親曲面予測圧縮を確認する。"""
import sqlite3,unittest
import numpy as np
from mls_quant_codec import decode_quant
from mls_parent_prediction import encode_prediction,decode_prediction,predict_plane
class PredictionTests(unittest.TestCase):
 def test_plane_prediction_and_parent_unchanged(self):
  rng=np.random.default_rng(90);xy=np.rint(rng.uniform(-15000,15000,(128,2))*4)/4;parent=np.column_stack((xy,100+xy[:,0]*.01+xy[:,1]*.02)).astype('<f4');original=parent.copy()
  xy=np.rint(rng.uniform(-8000,8000,(32,2))*4)/4;added=np.column_stack((xy,100+xy[:,0]*.01+xy[:,1]*.02)).astype('<f4');blob=encode_prediction(parent,added,.5);r=decode_prediction(parent,blob)
  self.assertEqual(sorted(map(tuple,r[:,:2])),sorted(map(tuple,added[:,:2])))
  self.assertLessEqual(float(abs(r[:,2]-(100+r[:,0]*.01+r[:,1]*.02)).max()),.2501);np.testing.assert_array_equal(parent,original)
 def test_saved_parent_and_xy(self):
  source={};resolved={}
  with sqlite3.connect('build/mls-hybrid-selected-v2/quality.sqlite3') as db:
   for e,parent,raw,b in db.execute('SELECT * FROM levels ORDER BY error DESC'):
    p=decode_quant(b);source[e]=np.vstack((source[parent],p)) if parent else p
  with sqlite3.connect('build/mls-parent-prediction-v2/quality.sqlite3') as db:
   for e,parent,codec,raw,b in db.execute('SELECT * FROM levels ORDER BY error DESC'):
    p=decode_prediction(resolved[parent],b) if codec=='MLP1' else decode_quant(b)
    resolved[e]=np.vstack((resolved[parent],p)) if parent else p
    np.testing.assert_array_equal(resolved[e][:,:2],source[e][:,:2])
    if parent:np.testing.assert_array_equal(resolved[e][:len(resolved[parent])],resolved[parent])
 def test_fast_prediction(self):
  rng=np.random.default_rng(80);xy=rng.uniform(-15000,15000,(128,2));p=np.column_stack((xy,100+.01*xy[:,0]+.02*xy[:,1]));q=rng.uniform(-8000,8000,(32,2))
  np.testing.assert_allclose(predict_plane(p,q),100+.01*q[:,0]+.02*q[:,1],atol=.0001)
 def test_fast_database(self):
  resolved={}
  with sqlite3.connect('build/mls-parent-prediction-v3/fast.sqlite3') as db:
   for e,parent,codec,raw,b in db.execute('SELECT * FROM levels ORDER BY error DESC'):
    p=decode_prediction(resolved[parent],b) if codec=='MLP2' else decode_quant(b)
    if parent:
     old=resolved[parent].copy();p=np.vstack((old,p));np.testing.assert_array_equal(p[:len(old)],old)
    resolved[e]=p
  self.assertEqual(len(resolved[25]),32729)
 def test_bad_magic(self):
  with self.assertRaises(ValueError):decode_prediction(np.zeros((0,3)),b'bad')
if __name__=='__main__':unittest.main()
