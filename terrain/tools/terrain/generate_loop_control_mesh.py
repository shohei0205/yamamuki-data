"""DEM適合の高さ補正を頂点座標へ直接反映したLoop制御メッシュを作る。"""
import argparse,hashlib,json,sqlite3,struct,zlib
from pathlib import Path
import numpy as np
ROOT=Path(__file__).resolve().parents[2]

def generate(source,hints,output):
 if output.exists():raise ValueError('既存の出力は上書きしません')
 src=sqlite3.connect(source.resolve().as_uri()+'?mode=ro',uri=True);fit=sqlite3.connect(hints.resolve().as_uri()+'?mode=ro',uri=True);db=sqlite3.connect(output)
 db.executescript('CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE tiles(level INTEGER,x INTEGER,y INTEGER,geometry BLOB,hints BLOB,mask BLOB,stats TEXT,PRIMARY KEY(level,x,y));')
 manifest=json.loads(src.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0]);reports={}
 for l,x,y,blob in fit.execute('SELECT level,x,y,data FROM tiles ORDER BY level,x,y'):
  g,h,m,meta=src.execute('SELECT geometry,hints,mask,stats FROM tiles WHERE level=? AND x=? AND y=?',(l,x,y)).fetchone();raw=zlib.decompress(g);hint=zlib.decompress(blob);magic,n,scale=struct.unpack_from('<4sIf',hint)
  if magic!=b'YLB1' or hashlib.sha256(raw).digest()!=hint[12:44]:raise ValueError('元メッシュと補正が一致しません')
  p=np.frombuffer(raw,dtype='<f4',count=n*3,offset=16).reshape(-1,3).copy();delta=np.frombuffer(hint,dtype='<i2',offset=44,count=n)*scale*np.frombuffer(hint,dtype='u1',offset=44+2*n,count=n)/255
  p[:,2]+=delta;new=zlib.compress(raw[:16]+p.tobytes()+raw[16+n*12:],9)
  # Loopは別の傾きヒントを読まない。互換用のゼロ列を高圧縮で格納する。
  empty=zlib.compress(struct.pack('<4sIf',b'YNH1',n,8.)+bytes(n*4),9)
  stats=json.loads(meta);stats.update(coordinateRole='loop-control',originalMaxErrorMeters=stats.get('maxErrorMeters'),maximumControlShiftMeters=float(abs(delta).max()),originalGeometryBytes=len(g),geometryBytes=len(new),bytes=len(new)+len(empty)+len(m));stats.pop('addedPositions',None)
  db.execute('INSERT INTO tiles VALUES(?,?,?,?,?,?,?)',(l,x,y,new,empty,m,json.dumps(stats,separators=(',',':'))))
  r=reports.setdefault(l,dict(level=l,tiles=0,vertices=0,originalGeometryBytes=0,controlGeometryBytes=0,bakedHintBytes=0,maskBytes=0))
  for k,v in dict(tiles=1,vertices=n,originalGeometryBytes=len(g),controlGeometryBytes=len(new),bakedHintBytes=len(blob),maskBytes=len(m)).items():r[k]+=v
 levels=sorted(reports);manifest.update(controlMesh=True,controlFitDivisions=4,controlReports=list(reports.values()),controlHeightOnly=True)
 manifest['levels']=[s for s in manifest['levels'] if s['level'] in levels]
 for area in manifest['reliefRegions']:
  area['levels']=[s for s in area['levels'] if s['level'] in levels]
  for spec in area['levels']:
   spec['addedPositions']=[];spec['bytes']=sum(db.execute('SELECT length(geometry)+length(hints)+length(mask) FROM tiles WHERE level=? AND x=? AND y=?',k).fetchone()[0] for k in spec['keys'])
 db.execute('INSERT INTO metadata VALUES("manifest",?)',(json.dumps(manifest),));db.commit();db.close();report=dict(sqliteBytes=output.stat().st_size,levels=list(reports.values()));output.with_suffix('.json').write_text(json.dumps(report,indent=2),encoding='utf-8');print(json.dumps(report))

if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--source',type=Path,default=ROOT/'build/terrain-regional-priority-levels-v5.sqlite3');p.add_argument('--hints',type=Path,default=ROOT/'build/terrain-loop-baked.sqlite3');p.add_argument('--output',type=Path,default=ROOT/'build/terrain-loop-control.sqlite3');a=p.parse_args();generate(a.source,a.hints,a.output)
