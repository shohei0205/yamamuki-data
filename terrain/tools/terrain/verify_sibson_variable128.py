"""可変サイズ全タイルの復元・補助情報・同品質の全共有辺を検証する。"""
import json,sqlite3,time
from pathlib import Path
from functools import lru_cache
import numpy as np
from sibson_variable128 import ROOT,LEVELS,restore,digest
from sibson_variable_server import Regional
from sibson_aux_surface import AuxiliarySibsonSurface
from sibson_region128 import write

def verify():
 started=time.time();db=sqlite3.connect(ROOT/'terrain.sqlite3',timeout=60);reports=[];checked=0;edge_count=0;maximum=0.;gradient_maximum=0.
 for region in ['fuji','yari','tsukuba']:
  adapter=Regional(db,region)
  @lru_cache(maxsize=256)
  def model(error,x,y):
   points=restore(adapter,error,x,y);blob=db.execute('SELECT auxiliary FROM tiles WHERE region=? AND error=? AND x=? AND y=?',(region,error,x,y)).fetchone()[0];return AuxiliarySibsonSurface(points,blob=blob)
  for error,side in LEVELS:
   count=int(128000/side)
   for y in range(count):
    for x in range(count):
     current=model(error,x,y);checked+=1
     for dx,dy in [(1,0),(0,1)]:
      if x+dx>=count or y+dy>=count:continue
      neighbor=model(error,x+dx,y+dy);q=np.column_stack((np.full(33,(x+1)*side),np.linspace(y*side,(y+1)*side,33))) if dx else np.column_stack((np.linspace(x*side,(x+1)*side,33),np.full(33,(y+1)*side)))
      difference=float(abs(current(q)-neighbor(q)).max());maximum=max(maximum,difference)
      edge_gradient=0.
      for axis in [0,1]:
       a=q.copy();b=q.copy();a[:,axis]=np.minimum(128000,a[:,axis]+1);b[:,axis]=np.maximum(0,b[:,axis]-1);den=a[:,axis]-b[:,axis];grad=float(abs((current(a)-current(b))/den-(neighbor(a)-neighbor(b))/den).max());gradient_maximum=max(gradient_maximum,grad);edge_gradient=max(edge_gradient,grad)
      if difference>1e-5 or edge_gradient>1e-5:raise ValueError(f'共有辺の不一致 {region} {error} {x} {y}')
      edge_count+=1
    write(ROOT/'verification-progress.json',dict(stage='verifying',region=region,error=error,row=y+1,rows=count,tiles=checked,edges=edge_count,maximumHeightDifference=maximum,maximumGradientDifference=gradient_maximum,elapsedSeconds=time.time()-started))
   size=db.execute('SELECT count(*),sum(length(blob)),sum(length(auxiliary)),min(json_extract(info,"$.points")),max(json_extract(info,"$.points")) FROM tiles WHERE region=? AND error=?',(region,error)).fetchone();reports.append(dict(region=region,error=error,side=side,tiles=size[0],geometryBytes=size[1],auxiliaryBytes=size[2],minimumPoints=size[3],maximumPoints=size[4]))
  model.cache_clear()
 if checked!=16383:raise ValueError('タイル数が不一致です')
 if db.execute('PRAGMA integrity_check').fetchone()[0]!='ok':raise ValueError('SQLite整合性検証が不合格')
 db.execute('PRAGMA wal_checkpoint(TRUNCATE)');result=dict(tiles=checked,edges=edge_count,maximumHeightDifference=maximum,maximumGradientDifference=gradient_maximum,sqliteBytes=(ROOT/'terrain.sqlite3').stat().st_size,levels=reports,elapsedSeconds=time.time()-started);write(ROOT/'verification.json',result);write(ROOT/'verification-progress.json',dict(stage='complete',tiles=checked,edges=edge_count,elapsedSeconds=time.time()-started));db.close()
if __name__=='__main__':
 try:verify()
 except Exception as e:write(ROOT/'verification-progress.json',dict(stage='failed',error=str(e)));raise
