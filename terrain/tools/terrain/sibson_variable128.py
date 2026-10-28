"""128 km共通曲面を品質別サイズのタイルへ分割し、補助情報も別BLOBへ保存する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json,sqlite3,time,struct,zlib,hashlib,sys
from pathlib import Path
from functools import lru_cache
import numpy as np
from scipy.spatial import cKDTree
from sibson_surface import SibsonSurface
from sibson_aux_surface import AuxiliarySibsonSurface
from mls_quant_codec import encode_quant,decode_quant
from sibson_region128 import write
SOURCE=Path('build/sibson-region128-v1');ROOT=Path('build/sibson-variable128-v1')
LEVELS=[(800.,128000.),(400.,64000.),(200.,32000.),(100.,16000.),(50.,8000.),(25.,4000.),(12.5,2000.)]
PARENTS={800:None,400:None,200:400,100:200,50:None,25:50,12.5:25}
HEADER=struct.Struct('<4sIII')
def digest(p):return hashlib.sha256(p.astype('<f4').tobytes()).digest()
def key_xy(p):
 q=np.rint(p[:,:2]/.0625).astype(np.int64);return (q[:,0]<<32)|q[:,1]
def encode_subset(points,parents):
 lookup={}
 for slot,parent in enumerate(parents):
  if len(parent)>=2**24:raise ValueError('親点番号が24bitを超えます')
  for index,key in enumerate(key_xy(parent)):lookup.setdefault(int(key),(slot,index))
 refs=np.full(len(points),0xffffffff,dtype='<u4');delta=np.zeros(len(points),dtype='<i4');fresh=[]
 for i,key in enumerate(key_xy(points)):
  found=lookup.get(int(key))
  if found is None:fresh.append(np.rint(points[i]/[.0625,.0625,.125]).astype('<i4'));continue
  slot,index=found;refs[i]=(slot<<24)|index;delta[i]=round((float(points[i,2])-float(parents[slot][index,2]))/.125)
 new=np.asarray(fresh,dtype='<i4').reshape(-1,3)
 body=refs.view('u1').reshape(-1,4).T.copy().tobytes()+delta.view('u1').reshape(-1,4).T.copy().tobytes()+new.view('u1').reshape(-1,4).T.copy().tobytes()
 return HEADER.pack(b'NSS2',len(points),len(parents),len(new))+zlib.compress(body,6)
def decode_subset(blob,parents):
 magic,n,np_,nn=HEADER.unpack_from(blob)
 if magic!=b'NSS2' or np_!=len(parents):raise ValueError('参照タイルが不一致です')
 raw=zlib.decompress(blob[HEADER.size:])
 if len(raw)!=8*n+12*nn:raise ValueError('差分の長さが不正です')
 refs=np.frombuffer(raw[:4*n],'u1').reshape(4,n).T.copy().view('<u4').ravel();delta=np.frombuffer(raw[4*n:8*n],'u1').reshape(4,n).T.copy().view('<i4').ravel();new=np.frombuffer(raw[8*n:],'u1').reshape(4,3*nn).T.copy().view('<i4').reshape(nn,3)
 fresh=refs==0xffffffff
 if fresh.sum()!=nn:raise ValueError('新規点数が不一致です')
 out=np.empty((n,3),np.float32);out[fresh]=new*[.0625,.0625,.125]
 for slot,parent in enumerate(parents):
  ids=np.flatnonzero((~fresh)&((refs>>24)==slot));indices=refs[ids]&0xffffff
  if len(indices) and indices.max()>=len(parent):raise ValueError('親点が範囲外です')
  out[ids]=parent[indices];out[ids,2]+=delta[ids]*.125
 if ((~fresh)&((refs>>24)>=len(parents))).any():raise ValueError('親タイル番号が範囲外です')
 return out

def restore(db,error,x,y,memo=None):
 if memo is None:memo={}
 key=(error,x,y)
 if key in memo:return memo[key]
 row=db.execute('SELECT parents,blob,sha256 FROM tiles WHERE error=? AND x=? AND y=?',key).fetchone()
 if row is None:raise ValueError(f'未生成タイル {key}')
 refs=json.loads(row[0]);p=decode_quant(row[1]) if not refs else decode_subset(row[1],[restore(db,*ref,memo) for ref in refs])
 if digest(p)!=row[2]:raise ValueError('点列の検査値が不一致です')
 memo[key]=p;return p

class CircleIndex:
 def __init__(self,model):
  self.model=model;radius=np.sqrt(model.circles[:,2]);groups=np.floor(np.log2(np.maximum(radius,1e-12))).astype(int);self.groups=[]
  for group in np.unique(groups):
   ids=np.flatnonzero(groups==group);self.groups.append((ids,cKDTree(model.circles[ids,:2]),float(radius[ids].max())))
 def extract(self,rect):
  model=self.model;center=(np.asarray(rect[:2])+rect[2:])/2;half=np.linalg.norm((np.asarray(rect[2:])-rect[:2])/2);hits=[]
  for ids,tree,radius in self.groups:
   candidates=ids[np.asarray(tree.query_ball_point(center,half+radius),dtype=int)]
   if not len(candidates):continue
   c=model.circles[candidates];closest=np.clip(c[:,:2],rect[:2],rect[2:]);hit=np.sum((c[:,:2]-closest)**2,axis=1)<=c[:,2]*(1+1e-12)+1e-8;hits.extend(candidates[hit])
  vertices=np.unique(model.tri[hits]);neighbors=[model.links[model.off[v]:model.off[v+1]] for v in vertices];vertices=np.unique(np.concatenate((vertices,*neighbors)));return np.column_stack((model.xy[vertices],model.z[vertices])).astype('<f4')

def generate(regions=None,errors=None):
 regions=regions or ['fuji','yari','tsukuba']
 ROOT.mkdir(parents=True,exist_ok=True);started=time.time();db=sqlite3.connect(ROOT/'terrain.sqlite3',timeout=30);db.execute('PRAGMA journal_mode=WAL');db.executescript('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE IF NOT EXISTS tiles(region TEXT,error REAL,x INTEGER,y INTEGER,side REAL,parents TEXT,blob BLOB,auxiliary BLOB,sha256 BLOB,info TEXT,PRIMARY KEY(region,error,x,y)) WITHOUT ROWID;')
 config=dict(span=128000,levels=[dict(error=e,side=s,tiles=int(128000/s)**2,parent=PARENTS[e]) for e,s in LEVELS],format='Sibson-variable128-v1',xyzQuantization=[.0625,.0625,.125],source=str(SOURCE),auxiliary='SAU1',halo=250,regions={r:json.loads((SOURCE/r/'source.json').read_text())['center'] for r in ['fuji','yari','tsukuba']})
 db.execute('INSERT OR REPLACE INTO metadata VALUES("config",?)',(json.dumps(config),));db.commit();total=3*sum(v['tiles'] for v in config['levels']);reports=[]
 for region in regions:
  source=sqlite3.connect((SOURCE/region/'terrain.sqlite3').resolve().as_uri()+'?mode=ro',uri=True)
  for error,side in LEVELS:
   if errors is not None and error not in errors:continue
   if db.execute("SELECT count(*) FROM tiles WHERE region=? AND error=?",(region,error)).fetchone()[0]==int(128000/side)**2:continue
   write(ROOT/'progress.json',dict(stage='indexing',region=region,error=error,side=side,total=total,done=db.execute('SELECT count(*) FROM tiles').fetchone()[0],elapsedSeconds=time.time()-started))
   blob,info=source.execute('SELECT blob,info FROM models WHERE error=?',(error,)).fetchone();global_points=decode_quant(blob);model=SibsonSurface(global_points);index=CircleIndex(model);count=int(128000/side);parent_error=PARENTS[error];parent_side=dict(LEVELS).get(parent_error);parent_db=sqlite3.connect(ROOT/'terrain.sqlite3',timeout=30)
   @lru_cache(maxsize=64)
   def parent_points(e,x,y):
    # 地域を固定した読み出し用の薄いSQLアダプター。
    class Regional:
     def execute(self,sql,params):return parent_db.execute(sql.replace('WHERE error=','WHERE region=? AND error='),(region,*params))
    return restore(Regional(),e,x,y)
   for y in sorted(range(count),key=lambda v:abs(v-(count-1)/2)):
    for x in sorted(range(count),key=lambda v:abs(v-(count-1)/2)):
     if db.execute('SELECT 1 FROM tiles WHERE region=? AND error=? AND x=? AND y=?',(region,error,x,y)).fetchone():continue
     bounds=np.array([x*side,y*side,(x+1)*side,(y+1)*side]);rect=np.clip(bounds+[-250,-250,250,250],0,128000);points=index.extract(rect);parents=[];arrays=[]
     if parent_error is not None:
      lo=np.maximum(0,np.floor(rect[:2]/parent_side).astype(int));hi=np.minimum(int(128000/parent_side)-1,np.floor(rect[2:]/parent_side).astype(int))
      for py in range(lo[1],hi[1]+1):
       for px in range(lo[0],hi[0]+1):parents.append([parent_error,int(px),int(py)]);arrays.append(parent_points(parent_error,int(px),int(py)))
      body=encode_subset(points,arrays);restored=decode_subset(body,arrays)
     else:body=encode_quant(points,xy=.0625,zstep=.125);restored=decode_quant(body)
     np.testing.assert_array_equal(restored,points);aux=AuxiliarySibsonSurface(restored);auxiliary=aux.auxiliary_blob();local=aux
     rng=np.random.default_rng(x+y*count+int(error*10));q=rng.uniform(rect[:2]+1e-4,rect[2:]-1e-4,(16,2));difference=float(abs(local(q)-model(q)).max())
     if difference>1e-5:raise ValueError(f'全域曲面と不一致 {region} {error} {x} {y}: {difference}')
     tileinfo=dict(region=region,error=error,x=x,y=y,side=side,bounds=bounds.tolist(),fieldBounds=rect.tolist(),points=len(points),geometryBytes=len(body),auxiliaryBytes=len(auxiliary),curveDifference=difference,parentTiles=parents)
     db.execute('INSERT INTO tiles VALUES(?,?,?,?,?,?,?,?,?,?)',(region,error,x,y,side,json.dumps(parents),body,auxiliary,digest(restored),json.dumps(tileinfo)));db.commit()
    write(ROOT/'progress.json',dict(stage='packing',region=region,error=error,side=side,row=y+1,rows=count,done=db.execute('SELECT count(*) FROM tiles').fetchone()[0],total=total,elapsedSeconds=time.time()-started))
   r=dict(region=region,error=error,side=side,tiles=count*count,sourceInfo=json.loads(info));reports.append(r);parent_points.cache_clear();parent_db.close();del index,model,global_points
  source.close()
 db.execute('PRAGMA wal_checkpoint(TRUNCATE)');actual=db.execute('SELECT count(*) FROM tiles').fetchone()[0]
 for region in regions:
  for error,side in LEVELS:
   if errors is None or error in errors:
    if db.execute('SELECT count(*) FROM tiles WHERE region=? AND error=?',(region,error)).fetchone()[0]!=int(128000/side)**2:raise ValueError('指定レベルの全タイルが揃っていません')
 if db.execute('PRAGMA integrity_check').fetchone()[0]!='ok':raise ValueError('SQLite整合性検査に失敗')
 write(ROOT/'generation-complete.json',dict(tiles=actual,total=total,elapsedSeconds=time.time()-started,levels=reports));write(ROOT/'progress.json',dict(stage='generated' if actual==total else 'partial',done=actual,total=total,elapsedSeconds=time.time()-started));db.close()

if __name__=='__main__':
 ROOT.mkdir(parents=True,exist_ok=True)
 guard=sqlite3.connect(ROOT/'generator-lock.sqlite3',timeout=1)
 guard.execute('CREATE TABLE IF NOT EXISTS run_guard(value INTEGER)')
 try:guard.execute('BEGIN EXCLUSIVE')
 except sqlite3.OperationalError:
  print('同じ出力先の生成が実行中です',flush=True);sys.exit(0)
 try:generate()
 except Exception as e:
  write(ROOT/'progress.json',dict(stage='failed',error=str(e)));raise
 finally:guard.close()
