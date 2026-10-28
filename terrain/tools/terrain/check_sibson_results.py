"""自然近傍の保存結果と描画用高さを照合する。"""
from pathlib import Path
import json
import numpy as np
from mls_quant_codec import decode_quant
from sibson_surface import SibsonSurface
from mls_lpf_dem import FilteredDem

def main():
 for r in ['fuji','yari','tsukuba']:
  for src,k in [('six-xy','natural-xy'),('six-more','natural-more')]:
   f=Path('build/sibson-surface-v2')/r/('50-'+k);i=json.loads((f/'info.json').read_text());p=decode_quant((f/'model.mlq1').read_bytes());old=decode_quant((Path('build/mls-xy-placement-v1')/r/('50-'+src)/'model.mlq1').read_bytes());np.testing.assert_array_equal(p[:,:2],old[:,:2]);assert i['samples']==11222500
   target=FilteredDem(r,i['sigmaMeters']).sample(p[:,:2]);assert np.max(abs(p[:,2]-target))<=.06251
   q=np.random.default_rng(62).uniform(-4000,4000,(1000,2));m=SibsonSurface(p);np.testing.assert_allclose(m(p[:,:2]),p[:,2],atol=1e-7)
   xy=((q+4000)/8000*1024).round().astype(int);gridq=xy/1024*8000-4000;field=np.fromfile(Path('build/mls-generated-preview/dem-regions')/r/('50-'+k+'-8000.bin'),'<f4').reshape(1025,1025,3);np.testing.assert_allclose(field[xy[:,1],xy[:,0],0],m(gridq),atol=.00013,rtol=0)
 print('6 モデル：XY 一致、目標 Z の量子化、全点検証件数、制御点通過、描画高さ一致')
if __name__=='__main__':main()
