"""全品質の復元と、同品質の全共有辺を検査して集計する。"""
from collections import OrderedDict
from pathlib import Path
import json,sqlite3,time,sys
import numpy as np
from sibson_region128 import ROOT,ERRORS,write
from sibson_region128_server import restore
from sibson_surface import SibsonSurface
from mls_quant_codec import decode_quant
from sibson_shared_codec import xy_keys
from mls_quant_codec import encode_quant

def export_distribution(folder):
 folder=Path(folder);source=json.loads((folder/'source.json').read_text(encoding='utf-8'))
 src=sqlite3.connect((folder/'terrain.sqlite3').resolve().as_uri()+'?mode=ro',uri=True)
 dest=sqlite3.connect(folder/'terrain-distribution.sqlite3')
 try:
  dest.executescript('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT) WITHOUT ROWID;CREATE TABLE IF NOT EXISTS tiles(x INTEGER,y INTEGER,error REAL,parent_error REAL,blob BLOB,sha256 BLOB,info TEXT,PRIMARY KEY(x,y,error)) WITHOUT ROWID;')
  metadata=dict(format='Sibson-NSP1-region128-v1',region=folder.name,spanMeters=128000,tileSizeMeters=8000,xyStepMeters=.0625,zStepMeters=.125,coordinateOrigin='地域の左上。東・南が正。global=local+center-64000',center=source['center'],independentErrors=[800,400,50],maximumReferenceDepth=3,source=source)
  dest.executemany('INSERT OR REPLACE INTO metadata VALUES(?,?)',[(k,json.dumps(v,ensure_ascii=False)) for k,v in metadata.items()])
  dest.executemany('INSERT OR REPLACE INTO tiles VALUES(?,?,?,?,?,?,?)',src.execute('SELECT x,y,error,parent_error,blob,sha256,info FROM tiles'))
  dest.commit();dest.execute('VACUUM')
  if dest.execute('PRAGMA integrity_check').fetchone()[0]!='ok':raise ValueError('配布DBの整合性検査が未達')
  if dest.execute('SELECT count(*) FROM tiles').fetchone()[0]!=1792:raise ValueError('配布DBの全品質が揃っていません')
 finally:dest.close();src.close()
 return (folder/'terrain-distribution.sqlite3').stat().st_size

def verify(root=ROOT,regions=None,errors=None):
 root=Path(root);reports=[];started=time.time();checked=0;distribution_sizes=[]
 regions=regions or ['fuji','yari','tsukuba'];errors=errors or ERRORS
 expected_parent={800:None,400:None,200:400,100:200,50:None,25:50,12.5:25}
 for region in regions:
  checkpoint=sqlite3.connect(root/region/'terrain.sqlite3',timeout=30)
  try:
   busy,_,_=checkpoint.execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchone()
   if busy:raise RuntimeError('WALの反映が完了していません')
  finally:checkpoint.close()
  db=sqlite3.connect((root/region/'terrain.sqlite3').resolve().as_uri()+'?mode=ro',uri=True)
  cache=OrderedDict();previous=None
  def model(x,y,e):
   nonlocal checked
   key=(x,y,e)
   if key in cache:cache.move_to_end(key);return cache[key]
   p=restore(db,x,y,e);result=SibsonSurface(p);cache[key]=result
   while len(cache)>40:cache.popitem(last=False)
   checked+=1
   return result
  for error in errors:
   row=db.execute('SELECT blob,info FROM models WHERE error=?',(error,)).fetchone()
   if row is None:raise ValueError('地域モデルが不足')
   p=decode_quant(row[0]);info=json.loads(row[1])
   if info['maxError']>error or info['samples']<1000000:raise ValueError('最大誤差の検査が未達')
   keys=xy_keys(p)
   if previous is not None and not np.isin(previous,keys).all():raise ValueError('地域モデルの親XYが不足')
   previous=keys
   rows=db.execute('SELECT x,y,parent_error,length(blob),info FROM tiles WHERE error=?',(error,)).fetchall()
   if len(rows)!=256:raise ValueError('256タイルが揃っていません')
   if {(r[0],r[1]) for r in rows}!={(x,y) for x in range(16) for y in range(16)}:raise ValueError('タイル座標が128 kmを覆っていません')
   maximum=0.;gradient=0.;edges=0;independent_bytes=0
   for x,y,pe,size,meta in rows:
    if pe!=expected_parent[error]:raise ValueError('親レベルが仕様と違います')
    a=model(x,y,error)
    independent_bytes+=len(encode_quant(np.column_stack((a.xy,a.z)),xy=.0625,zstep=.125))
    for dx,dy in [(1,0),(0,1)]:
     if x+dx>=16 or y+dy>=16:continue
     b=model(x+dx,y+dy,error);t=np.linspace(0,8000,257)
     q=np.column_stack((np.full(257,(x+1)*8000),y*8000+t)) if dx else np.column_stack((x*8000+t,np.full(257,(y+1)*8000)))
     maximum=max(maximum,float(abs(a(q)-b(q)).max()))
     for axis in [0,1]:
      u=q.copy();v=q.copy();u[:,axis]=np.minimum(128000,u[:,axis]+1);v[:,axis]=np.maximum(0,v[:,axis]-1)
      ga=(a(u)-a(v))/(u[:,axis]-v[:,axis]);gb=(b(u)-b(v))/(u[:,axis]-v[:,axis]);gradient=max(gradient,float(abs(ga-gb).max()))
     edges+=1
    write(root/'verification-progress.json',dict(stage='verifying',region=region,error=error,checkedModels=checked,edges=edges,heightGap=maximum,gradientGap=gradient,elapsedSeconds=time.time()-started))
   if maximum>1e-5 or gradient>1e-5:raise ValueError(f'同品質の境界が不一致 {region} {error}: {maximum} {gradient}')
   reports.append(dict(region=region,error=error,uniqueRegionPoints=len(p),tilePoints=sum(json.loads(r[4])['points'] for r in rows),haloPoints=sum(json.loads(r[4])['haloPoints'] for r in rows),blobBytes=sum(r[3] for r in rows),independentBlobBytes=independent_bytes,savingPercent=100*(1-sum(r[3] for r in rows)/independent_bytes),maxDemError=info['maxError'],demSamples=info['samples'],edges=edges,heightGap=maximum,gradientGap=gradient))
  db.close()
  if errors==ERRORS:distribution_sizes.append(dict(region=region,bytes=export_distribution(root/region)))
 write(root/'verification.json',dict(passed=True,levels=reports,sqliteBytes=sum((root/r/'terrain.sqlite3').stat().st_size for r in regions),distributionSqlite=distribution_sizes,totalBlobBytes=sum(r['blobBytes'] for r in reports),totalIndependentBlobBytes=sum(r['independentBlobBytes'] for r in reports),elapsedSeconds=time.time()-started))
 write(root/'verification-progress.json',dict(stage='complete',levels=len(reports),edges=sum(r['edges'] for r in reports),elapsedSeconds=time.time()-started))
 if len(regions)==3 and errors==ERRORS:
  write(root/'all-work-complete.json',dict(dataGenerated=True,allTileBlobsRestored=True,allSameQualityEdgesVerified=True,distributionSqlite=distribution_sizes,browserLogicTest='browser-fixture.json',realBrowserOverview='fuji-real-overview.png'))

def wait_and_verify():
 while True:
  ready=True
  for region in ['fuji','yari','tsukuba']:
   path=ROOT/region/'progress.json'
   try:state=json.loads(path.read_text(encoding='utf-8'))
   except (FileNotFoundError,json.JSONDecodeError):ready=False;continue
   if state['stage']=='failed':raise RuntimeError(region+'の生成が停止しています')
   if state['stage']!='complete':ready=False
  if ready:break
  write(ROOT/'verification-progress.json',dict(stage='waiting-for-generation',updatedAt=time.time()));time.sleep(10)
 verify()

if __name__=='__main__':
 try:
  if '--wait' in sys.argv:wait_and_verify()
  else:verify()
 except Exception as e:
  write(ROOT/'verification-progress.json',dict(stage='failed',error=str(e)));raise
