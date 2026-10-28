"""タイルごとに元DEMへ独立適合し、接合比較用の場を生成する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS', '1')
import concurrent.futures
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import numpy as np
from mls_lpf_dem import FilteredDem
from raw_dem_reference import RawDem, center_for
from mls_quant_codec import encode_quant, decode_quant
from sibson_surface import SibsonSurface
from sibson_tile_seams import TILES, expanded, STEP

ROOT = Path('build/sibson-independent-seams-v1')
VIEW = Path('build/mls-generated-preview/dem-natural-independent-seams')
ERRORS = [800, 400, 200, 100, 50, 25, 12.5]
NO_OVERLAP = os.environ.get('SIBSON_SEAMS_NO_OVERLAP') == '1'
BOUNDARY_AUDIT = os.environ.get('SIBSON_SEAMS_BOUNDARY_AUDIT') == '1'
if NO_OVERLAP:
 ROOT = Path('build/sibson-no-overlap-seams-v2' if BOUNDARY_AUDIT else 'build/sibson-no-overlap-seams-v1')
 VIEW = Path('build/mls-generated-preview/dem-natural-no-overlap-seams-v2' if BOUNDARY_AUDIT else 'build/mls-generated-preview/dem-natural-no-overlap-seams')
 def expanded(bounds):
  return list(bounds)

def write(path, value):
 path.write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding='utf-8', newline='\n')

def fit(raw, filtered, rect, error, folder):
 # 元PNGの全画素中心を、このタイルの余白を含む矩形へ限定する。
 chunks=[]; heights=[]
 for q,h,_ in raw.native_rows(64):
  keep=(q[:,0]>=rect[0])&(q[:,0]<=rect[2])&(q[:,1]>=rect[1])&(q[:,1]<=rect[3])
  chunks.append(q[keep]);heights.append(h[keep])
 q=np.vstack(chunks);truth=np.concatenate(heights);native_samples=len(q)
 if BOUNDARY_AUDIT:
  from raw_dem_reference import STEP as source_step
  edge_x=np.linspace(rect[0],rect[2],int(np.ceil((rect[2]-rect[0])/source_step))+1)
  edge_y=np.linspace(rect[1],rect[3],int(np.ceil((rect[3]-rect[1])/source_step))+1)
  boundary=np.unique(np.vstack((np.column_stack((edge_x,np.full(len(edge_x),rect[1]))),np.column_stack((edge_x,np.full(len(edge_x),rect[3]))),np.column_stack((np.full(len(edge_y),rect[0]),edge_y)),np.column_stack((np.full(len(edge_y),rect[2]),edge_y)))),axis=0)
  q=np.vstack((q,boundary));truth=np.concatenate((truth,raw.sample(boundary)))
 # 隣接タイルの点を参照せず、各矩形内の粗い初期点から開始する。
 x=np.linspace(rect[0],rect[2],int((rect[2]-rect[0])/1000)+1)
 y=np.linspace(rect[1],rect[3],int((rect[3]-rect[1])/1000)+1)
 xx,yy=np.meshgrid(x,y);xy=np.column_stack((xx.ravel(),yy.ravel()))
 history=[]
 for iteration in range(40):
  blob=encode_quant(np.column_stack((xy,filtered.sample(xy))),xy=.0625,zstep=.125)
  points=decode_quant(blob);model=SibsonSurface(points)
  best=np.zeros(len(model.tri));positions=np.zeros((len(best),2));maximum=0.;squared=0.
  for start in range(0,len(q),65536):
   query=q[start:start+65536];err=abs(model(query)-truth[start:start+65536])
   maximum=max(maximum,float(err.max()));squared+=float(err@err)
   ids=model.triangulation.find_simplex(query)
   if np.any(ids<0):raise ValueError('評価点がタイル曲面の外側です')
   np.maximum.at(best,ids,err)
   selected=np.flatnonzero((err>=best[ids])&(err>error*.98))
   positions[ids[selected]]=query[selected]
  row=dict(iteration=iteration,points=len(points),maxError=maximum,rmse=float(np.sqrt(squared/len(q))),samples=len(q),nativeSamples=native_samples,boundarySamples=len(q)-native_samples)
  history.append(row);write(folder/'iterations.json',history);print(json.dumps(row),flush=True)
  if maximum<=error:return points,blob,row
  ids=np.flatnonzero(best>error*.98);ids=ids[np.argsort(-best[ids],kind='stable')[:10000]]
  updated=np.unique(np.rint(np.vstack((points[:,:2],positions[ids]))*16)/16,axis=0)
  if len(updated)==len(points):raise RuntimeError('追加可能な候補がありません')
  xy=updated
 raise RuntimeError('40回で誤差条件を満たしませんでした')

def worker(region):
 folder=ROOT/region;folder.mkdir(parents=True,exist_ok=True)
 dest=VIEW/region;dest.mkdir(parents=True,exist_ok=True)
 raw=RawDem('build/terrain-cache',center_for(region));entries=[]
 db=sqlite3.connect(folder/'tiles.sqlite3')
 db.execute('CREATE TABLE IF NOT EXISTS tiles (tile TEXT,error REAL,blob BLOB,info TEXT,PRIMARY KEY(tile,error)) WITHOUT ROWID')
 for error in ERRORS:
  # 同じLPFを使い、差をタイルごとの独立した点配置に限定する。
  source=Path('build/sibson-nested-v1')/region/f'{error:g}-natural-nested{error:g}'/'model-info.json'
  sigma=json.loads(source.read_text(encoding='utf-8'))['sigmaMeters'];filtered=FilteredDem(region,sigma)
  for name,bounds in TILES.items():
   key=f'{error:g}-{name}';work=folder/key;work.mkdir(exist_ok=True)
   rect=expanded(bounds);saved=db.execute('SELECT blob,info FROM tiles WHERE tile=? AND error=?',(name,error)).fetchone()
   if saved:blob,info=saved;info=json.loads(info);points=decode_quant(blob)
   else:
    points,blob,audit=fit(raw,filtered,rect,error,work)
    core=(points[:,0]>=bounds[0])&(points[:,0]<=bounds[2])&(points[:,1]>=bounds[1])&(points[:,1]<=bounds[3])
    info=dict(key=key,error=error,tile=name,bounds=bounds,fieldBounds=rect,step=STEP,
     nx=int(round((rect[2]-rect[0])/STEP)),ny=int(round((rect[3]-rect[1])/STEP)),
     points=len(points),corePoints=int(core.sum()),haloPoints=int((~core).sum()),
     maxOutsideDistanceMeters=float(np.linalg.norm(np.maximum(np.maximum(np.asarray(bounds[:2])-points[:,:2],points[:,:2]-np.asarray(bounds[2:])),0),axis=1).max()),
     bytes=len(blob),storedBytes=len(blob),parentError=None,independentlyFitted=True,sigmaMeters=sigma,audit=audit)
    db.execute('INSERT INTO tiles VALUES(?,?,?,?)',(name,error,blob,json.dumps(info)));db.commit()
   if not (dest/(key+'.bin')).exists():
    model=SibsonSurface(points);x,y=np.meshgrid(np.linspace(rect[0],rect[2],info['nx']+1),np.linspace(rect[1],rect[3],info['ny']+1))
    h=model(np.column_stack((x.ravel(),y.ravel()))).reshape(info['ny']+1,info['nx']+1)
    gy,gx=np.gradient(h,STEP,edge_order=2);np.stack((h,gx,gy),axis=2).astype('<f4').tofile(dest/(key+'.bin'))
   entries.append(info);write(folder/'progress.json',dict(stage='generating',completed=len(entries),total=35,last=info))
   write(dest/'manifest.json',dict(region=region,maxWidth=2000,independentlyFitted=True,tiles=entries))
 db.close();write(folder/'progress.json',dict(stage='complete',completed=35,total=35))

def main():
 ROOT.mkdir(exist_ok=True);VIEW.mkdir(exist_ok=True)
 for src,dst in [('sibson_seams.html','index.html'),('sibson_seams.js','seams.js')]:
  text=(Path('tools/terrain')/src).read_text(encoding='utf-8')
  if src.endswith('.js'):
   text=text.replace("uniformLevel=active.length<2||active.every(t=>t.info.error===active[0].info.error)","uniformLevel=active.length<2")
  else:
   text=text.replace('自然近傍・同品質／混合品質／境界の一部分の接合','自然近傍・独立生成タイルの接合')
   text=text.replace('同品質は同じ地域モデルを分割し、補間の影響範囲を余白に保存。独立に生成した隣接地域の接合は別検証です。余白にも親XY包含を保ち、800・400・50 mを独立保存しています。','各タイルは元PNGの全画素中心へ個別に適合しています。点を隣接タイルから継承せず、同品質でも境界帯で混合します。余白は2 km。今回は全品質を独立保存し、差分圧縮は未適用です。')
  if NO_OVERLAP:
   if src.endswith('.js'):
    text=text.replace('function sample(tile,x,y)', 'function sampleRaw(tile,x,y)')
    text=text.replace('function evaluate(owner,x,y,joined=', 'function evaluateAnalytic(owner,x,y,joined=')
    text+=(Path('tools/terrain/sibson_no_overlap.js').read_text(encoding='utf-8'))
    text=text.replace("'保存点の最大領域外距離 '","'保存余白なし。領域外の表示データは読み込みません。保存点の最大領域外距離 '")
   else:
    text=text.replace('自然近傍・独立生成タイルの接合','自然近傍・余白なし独立生成タイル')
    text=text.replace('境界帯で共通化','境界の傾きから外挿して接合')
    text=text.replace('接合帯・片側','補正帯・片側')
    text=text.replace('余白は2 km。','余白は0 m。領域外の点・場は保存しません。接合ONは端の高さと傾きから実行時に線形外挿し、補正帯で混ぜる試験です。隣の地形を正確に復元する処理ではありません。')
    text=text.replace('同品質でも境界帯で混合します。','同品質でも境界差を補正します。「接合なし」で無補正の状態と比較できます。')
    text=text.replace('隣の地形を正確に復元する処理ではありません。','隣の地形を正確に復元する処理ではありません。広い補正帯で変形が大きいため、単純外挿の接合は非採用・比較用です。')
    if BOUNDARY_AUDIT:
     text=text.replace('余白は0 m。','余白は0 m。タイル端の誤差も生成時に検査済み。')
    text=text.replace('<option value="1">','<option value="1">',1).replace('<option value="0">接合なし','<option value="0" selected>接合なし',1)
  (VIEW/dst).write_text(text,encoding='utf-8',newline='\n')
 def run(region):
  with (ROOT/(region+'.log')).open('w',encoding='utf-8') as log:
   p=subprocess.Popen([sys.executable,'-X','utf8',__file__,region],stdout=log,stderr=subprocess.STDOUT)
   write(ROOT/(region+'.launch.json'),dict(pid=p.pid))
   if p.wait():raise RuntimeError(region+' の独立生成に失敗')
 write(ROOT/'progress.json',dict(stage='generating'))
 with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:list(pool.map(run,['fuji','yari','tsukuba']))
 write(ROOT/'progress.json',dict(stage='complete'))

if __name__=='__main__':
 if len(sys.argv)>1:worker(sys.argv[1])
 else:main()
