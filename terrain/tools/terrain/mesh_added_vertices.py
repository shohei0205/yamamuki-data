"""固定50mメッシュとの差分頂点を比較データへ記録する。"""
import json,sqlite3,struct,zlib
from pathlib import Path
import numpy as np

ROOT=Path(__file__).resolve().parents[2]

def region_points(db,bounds):
 left,right,bottom,top=bounds;points=set()
 for x,y,blob in db.execute('SELECT x,y,geometry FROM tiles WHERE level=3 AND x>=? AND x<? AND y>=? AND y<?',(int(left/8000),int(right/8000),int(bottom/8000),int(top/8000))):
  raw=zlib.decompress(blob);magic,n,_,nv,ni=struct.unpack_from('<4sHHII',raw)
  if magic!=b'YNT2':raise ValueError('比較データはYNT2が必要です')
  p=np.frombuffer(raw,dtype='<f4',count=nv*3,offset=16).reshape(-1,3).astype(float);p[:,:2]+=[x*8000,y*8000]
  points.update(map(tuple,p.tolist()))
 return points

def update(paths=None):
 fixed=ROOT/'build/terrain-relief-fixed-l3.sqlite3'
 if not fixed.exists():return
 with sqlite3.connect(fixed) as base:
  meta=json.loads(base.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
  references={r['id']:region_points(base,r['bounds']) for r in meta['reliefRegions']}
 for path in paths or sorted((ROOT/'build').glob('terrain-relief-*-l3.sqlite3')):
  with sqlite3.connect(path) as db:
   row=db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()
   if not row:continue
   meta=json.loads(row[0])
   for r in meta.get('reliefRegions',[]):
    if r['id'] not in references:continue
    # 二段階生成は実際の初期メッシュとの差分を保持する。
    if r.get('settings',{}).get('mode') in ['ridge1024','priority1024']:
     r['addedReference']='固定50 mの初期メッシュ'
    else:
     current=region_points(db,r['bounds']);reference=references[r['id']]
     r['addedPositions']=[list(p) for p in sorted(current-reference)]
     r['addedReference']='固定50 m比較メッシュとの差分'
     r['comparisonAddedVertices']=len(current-reference)
     r['comparisonRemovedVertices']=len(reference-current)
    print(path.stem,r['id'],len(r.get('addedPositions',[])))
   db.execute('UPDATE metadata SET value=? WHERE key="manifest"',(json.dumps(meta),))

if __name__=='__main__':update()
