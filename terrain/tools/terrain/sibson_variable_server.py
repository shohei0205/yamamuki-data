"""可変サイズのSibsonタイルを復元し、表示用の高さ・傾きを返す。"""
import json,sqlite3,struct,threading,hashlib
from contextlib import closing
from collections import OrderedDict
from pathlib import Path
import numpy as np
from sibson_variable128 import ROOT,LEVELS,restore
from sibson_aux_surface import AuxiliarySibsonSurface
lock=threading.Lock();slots=threading.Semaphore(2);cache=OrderedDict();models=OrderedDict()
def database():return closing(sqlite3.connect((ROOT/'terrain.sqlite3').resolve().as_uri()+'?mode=ro',uri=True,timeout=30))
class Regional:
 def __init__(self,db,region):self.db=db;self.region=region
 def execute(self,sql,params):return self.db.execute(sql.replace('WHERE error=','WHERE region=? AND error='),(self.region,*params))
def manifest(region):
 with database() as db:
  config=json.loads(db.execute('SELECT value FROM metadata WHERE key="config"').fetchone()[0]);tiles=[json.loads(row[0]) for row in db.execute('SELECT info FROM tiles WHERE region=? ORDER BY error DESC,y,x',(region,)) if float(json.loads(row[0])['error']) in {e for e,_ in LEVELS}]
 try:progress=json.loads((ROOT/'progress.json').read_text())
 except (FileNotFoundError,json.JSONDecodeError):progress={}
 try:verification=json.loads((ROOT/'verification-progress.json').read_text())
 except (FileNotFoundError,json.JSONDecodeError):verification={}
 return json.dumps(dict(config=config,tiles=tiles,progress=progress,verification=verification)).encode()
def field(region,error,x,y,n):
 key=(region,error,x,y,n)
 with lock:
  if key in cache:cache.move_to_end(key);return cache[key]
 with slots,database() as db:
  row=db.execute('SELECT auxiliary,sha256,info FROM tiles WHERE region=? AND error=? AND x=? AND y=?',(region,error,x,y)).fetchone()
  if row is None:raise ValueError('このタイルは生成待ちです')
  info=json.loads(row[2]);model_key=(region,error,x,y,row[1])
  with lock:model=models.get(model_key)
  if model is None:
   points=restore(Regional(db,region),error,x,y);model=AuxiliarySibsonSurface(points,blob=row[0])
   with lock:
    if model.aux_bytes<=64*1024*1024:
     models[model_key]=model
     while len(models)>32 or sum(m.aux_bytes for m in models.values())>64*1024*1024:models.popitem(last=False)
  rect=np.asarray(info['fieldBounds']);bounds=info['bounds'];xs=np.unique(np.r_[rect[0],np.linspace(bounds[0],bounds[2],n+1),rect[2]]);ys=np.unique(np.r_[rect[1],np.linspace(bounds[1],bounds[3],n+1),rect[3]]);nx=len(xs)-1;ny=len(ys)-1;xx,yy=np.meshgrid(xs,ys);q=np.column_stack((xx.ravel(),yy.ravel()));h=model(q);grad=[]
  for axis in [0,1]:
   a=q.copy();b=q.copy();a[:,axis]=np.minimum(128000,a[:,axis]+1);b[:,axis]=np.maximum(0,b[:,axis]-1);grad.append((model(a)-model(b))/(a[:,axis]-b[:,axis]))
  info.update(n=n,nx=nx,ny=ny);header=json.dumps(info).encode();body=struct.pack('<I',len(header))+header+np.column_stack((h,*grad)).astype('<f4').tobytes()
 with lock:
  cache[key]=body
  while len(cache)>96 or sum(len(v) for v in cache.values())>96*1024*1024:cache.popitem(last=False)
 return body
