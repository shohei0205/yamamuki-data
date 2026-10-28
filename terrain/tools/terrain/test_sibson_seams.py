"""接合の重み勾配と、保存タイルの曲面一致を検証する。"""
import json,sqlite3,unittest
from pathlib import Path
import numpy as np
from sibson_tile_seams import blend,TILES
from sibson_shared_codec import load_shared,xy_keys
from mls_quant_codec import decode_quant

class BlendTest(unittest.TestCase):
 def plane(self,c,gx=0,gy=0):
  return lambda q:np.column_stack((c+q[:,0]*gx+q[:,1]*gy,np.full(len(q),gx),np.full(len(q),gy)))
 def test_open_edge_and_same_surface(self):
  q=np.array([[0,0],[20,100],[-100,-100]],float);f=self.plane(3000,.1,.2)
  np.testing.assert_allclose(blend(q,[(TILES['nw'],f)]),f(q),atol=1e-10)
  np.testing.assert_allclose(blend(q,[(TILES['nw'],f),(TILES['ne'],f)]),f(q),atol=1e-10)
 def test_gradient_includes_weight_derivative(self):
  tiles=[(TILES['large'],self.plane(3000)),(TILES['ne'],self.plane(3100)),(TILES['se'],self.plane(3200))]
  q=np.array([[0,0],[1,2],[-10,100],[50,-100]],float);values=blend(q,tiles)
  for axis in range(2):
   shift=np.zeros(2);shift[axis]=.01
   numerical=(blend(q+shift,tiles)[:,0]-blend(q-shift,tiles)[:,0])/.02
   np.testing.assert_allclose(values[:,axis+1],numerical,atol=1e-7,rtol=0)

class TileTest(unittest.TestCase):
 def test_real_tiles(self):
  root=Path('build/sibson-seams-v2')
  for region in ['fuji','yari','tsukuba']:
   entries=json.loads((root/region/'manifest.json').read_text())
   self.assertEqual(len(entries),35)
   with sqlite3.connect((root/region/'tiles.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as db:
    for error in [800,400,200,100,50,25,12.5]:
     models={}
     for name in TILES:
      p,model,stats=load_shared(db,region+':'+name,error)
      expected=decode_quant((root/region/f'{error:g}-{name}.mlq1').read_bytes());np.testing.assert_array_equal(p,expected)
      info=next(e for e in entries if e['error']==error and e['tile']==name)
      self.assertLess(info['maxDifferenceFromUnsplit'],1e-5)
      self.assertEqual(info['points'],len(p));self.assertLessEqual(len(stats['levels']),3);models[name]=model
     for a,b,lo,hi,axis in [('nw','ne',0,8000,0),('sw','se',-8000,0,0),('nw','sw',-8000,0,1),('large','ne',0,8000,0),('large','se',-8000,0,0)]:
      t=np.linspace(lo,hi,257);q=np.column_stack((np.zeros(len(t)),t)) if axis==0 else np.column_stack((t,np.zeros(len(t))))
      np.testing.assert_allclose(models[a](q),models[b](q),atol=1e-5,rtol=0)

if __name__=='__main__':unittest.main()
