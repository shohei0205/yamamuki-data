"""可変サイズ128 kmタイルの制御点パケット。曲面の密な標高配列は作らない。"""
import json, threading, time
from pathlib import Path
import numpy as np
from sibson_variable_server import database, Regional
from sibson_variable128 import restore, LEVELS
from sibson_gpu_controls import pack
from sibson_aux_surface import AuxiliarySibsonSurface

ROOT=Path('build/sibson-gpu-variable-v1')
slots=threading.Semaphore(2)
locks={};guard=threading.Lock()

def packet(region,error,x,y):
 if error not in dict(LEVELS):raise ValueError('不正な品質')
 side=dict(LEVELS)[error];count=round(128000/side)
 if not (0<=x<count and 0<=y<count):raise ValueError('タイルが範囲外です')
 key=(region,error,x,y)
 with guard:lock=locks.setdefault(key,threading.Lock())
 path=ROOT/region/f'{error:g}'/f'{x}-{y}-fast.bin'
 legacy=path.with_name(f'{x}-{y}.bin')
 with lock:
  if path.exists():return path.read_bytes()
  # 過去の過剰な探索木は保持するが配信しない。小さい既存パケットは再利用。
  if legacy.exists() and legacy.stat().st_size<=1048576:return legacy.read_bytes()
  with slots,database() as db:
   started=time.perf_counter()
   row=db.execute('SELECT info FROM tiles WHERE region=? AND error=? AND x=? AND y=?',key).fetchone()
   if row is None:raise ValueError('元タイルがありません')
   info=json.loads(row[0]);points=restore(Regional(db,region),error,x,y).astype(float)
   # 地域の外周だけ鏡像点を足し、凸包上の無限セルを有限にする。
   # 内部タイルの補間条件は変更しない。
   variants=[points]
   for axis in [0,1]:
    if info['bounds'][axis]==0:
     for p in list(variants):
      q=p.copy();q[:,axis]=-q[:,axis];variants.append(q)
    if info['bounds'][axis+2]==128000:
     for p in list(variants):
      q=p.copy();q[:,axis]=256000-q[:,axis];variants.append(q)
   points=np.unique(np.vstack(variants),axis=0)
   origin=(np.array(info['bounds'][:2])+info['bounds'][2:])/2
   points[:,:2]-=origin
   bounds=np.array(info['fieldBounds'])-np.tile(origin,2)
   # ±1 m の勾配評価も地域外周で有限にする。
   model=AuxiliarySibsonSurface(points);close_cells(model)
   data,meta=pack(points,np.zeros((len(points),2)),bounds,dict(region=region,error=error,x=x,y=y,side=side,origin=origin.tolist(),coreBounds=info['bounds'],worldFieldBounds=info['fieldBounds'],curve='original-Sibson',outerBoundary='mirror',sourceGeometryBytes=info['geometryBytes']),started,model=model,max_candidates=32,max_depth=8)
   if meta['unsafeLeaves']:raise ValueError('補間範囲に無限Voronoiセルが残っています')
   path.parent.mkdir(parents=True,exist_ok=True);temporary=path.with_suffix('.tmp');temporary.write_bytes(data);temporary.replace(path)
   path.with_suffix('.json').write_text(json.dumps(meta,ensure_ascii=False,indent=2),encoding='utf-8')
   return data


def close_cells(model):
 # 有限セルが取れない外周点は半平面で構成する。十分遠い作業枠で閉じ、
 # 枠を広げたときの補間値不変性を検証する。制御点自体は追加しない。
 cells=[];offset=[0];radius=float(np.ptp(model.xy,axis=0).max())*8
 for i in range(len(model.xy)):
  cell=model.cells[model.cell_off[i]:model.cell_off[i+1]]
  if len(cell)==0:
   cell=np.array([[-radius,-radius],[radius,-radius],[radius,radius],[-radius,radius]])
   ids=model.links[model.off[i]:model.off[i+1]]
   for delta in model.xy[ids]-model.xy[i]:
    distance=cell@delta-float(delta@delta)*.5;out=[]
    for j in range(len(cell)):
     previous=(j-1)%len(cell);a,b=distance[previous],distance[j]
     if (a<=0)!=(b<=0):out.append(cell[previous]+(cell[j]-cell[previous])*(a/(a-b)))
     if b<=0:out.append(cell[j])
    cell=np.asarray(out).reshape(-1,2)
    if not len(cell):raise ValueError('Voronoiセルの再構成に失敗しました')
  cells.extend(cell);offset.append(len(cells))
 model.cells=np.ascontiguousarray(cells,dtype=float);model.cell_off=np.asarray(offset,dtype=np.int32)
