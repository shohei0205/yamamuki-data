"""表示場の接合前後のDEM誤差を、境界周辺の固定乱数点で比較する。"""
import json
from pathlib import Path
import numpy as np
from scipy.ndimage import map_coordinates
from raw_dem_reference import RawDem,center_for
from sibson_tile_seams import blend

ROOT=Path('build/sibson-independent-seams-v1')
VIEW=Path('build/mls-generated-preview/dem-natural-independent-seams')
EXTRAPOLATE=False

def main():
 results=[]
 for region in ['fuji','yari','tsukuba']:
  raw=RawDem('build/terrain-cache',center_for(region));manifest=json.loads((VIEW/region/'manifest.json').read_text());cache={}
  def field(name,error):
   key=(name,error)
   if key not in cache:
    info=next(t for t in manifest['tiles'] if t['tile']==name and t['error']==error)
    data=np.fromfile(VIEW/region/(info['key']+'.bin'),dtype='<f4').reshape(info['ny']+1,info['nx']+1,3)
    def evaluate(q):
     uv=(q-np.asarray(info['fieldBounds'][:2]))/info['step']
     values=np.column_stack([map_coordinates(data[:,:,k],[uv[:,1],uv[:,0]],order=1,prefilter=False,mode='nearest') for k in range(3)])
     if EXTRAPOLATE:
      clipped=np.clip(q,info['fieldBounds'][:2],info['fieldBounds'][2:])
      values[:,0]+=np.sum((q-clipped)*values[:,1:],axis=1)
     return values
    cache[key]=(info['bounds'],evaluate)
   return cache[key]
  cases=[(f'same-{e}', [('nw',e),('ne',e),('sw',e),('se',e)]) for e in [800,400,200,100,50,25,12.5]]
  cases += [('mixed',[('nw',200),('ne',50),('sw',25),('se',100)]),('tee',[('large',200),('ne',25),('se',100)]),('half',[('large',200),('ne',25)]),('extreme',[('large',800),('ne',12.5)])]
  for label,selection in cases:
   tiles=[field(*key) for key in selection]
   for width in [250,2000]:
    rng=np.random.default_rng(8763);errors=[];before=[];gaps=[]
    # 各所有領域内の共有境界に近い点を評価する。未読込側は含めない。
    for bounds,evaluator in tiles:
     q=rng.uniform(bounds[:2],bounds[2:],(16384,2));near=np.zeros(len(q),bool)
     for other,_ in tiles:
      if other is bounds:continue
      distance=np.linalg.norm(np.maximum(np.maximum(np.asarray(other[:2])-q,q-np.asarray(other[2:])),0),axis=1)
      near|=distance<width
     q=q[near]
     if not len(q):continue
     truth=raw.sample(q);unjoined=evaluator(q)[:,0];joined=blend(q,tiles,width)[:,0]
     errors.extend(abs(joined-truth));before.extend(abs(unjoined-truth));gaps.extend(abs(joined-unjoined))
    row=dict(region=region,case=label,width=width,samples=len(errors),beforeMax=float(max(before)),afterMax=float(max(errors)),afterRmse=float(np.sqrt(np.mean(np.square(errors)))),maxChange=float(max(gaps)))
    results.append(row)
 (ROOT/'blend-audit.json').write_text(json.dumps(dict(domain='fixed-random-samples-in-boundary-band; rendered-31.25m-fields; not-a-maximum-error-guarantee',results=results),indent=2),encoding='utf-8',newline='\n')
 print(json.dumps(dict(cases=len(results),maximumChange=max(r['maxChange'] for r in results))))

if __name__=='__main__':main()
