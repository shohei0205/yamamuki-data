"""補助情報を読み込み時に作る、同じ形状BLOBの配布DBを作る。"""
import json,sqlite3
from sibson_variable128 import ROOT,SOURCE

def export():
 src=sqlite3.connect(ROOT/'terrain.sqlite3');path=ROOT/'terrain-distribution.sqlite3'
 if path.exists():raise ValueError('既存の配布DBは上書きしません')
 audits={}
 for region in ['fuji','yari','tsukuba']:
  with sqlite3.connect(SOURCE/region/'terrain.sqlite3') as db:audits[region]={str(row[0]):json.loads(row[1]) for row in db.execute('select error,info from models')}
 src.execute('INSERT OR REPLACE INTO metadata VALUES("source_audits",?)',(json.dumps(audits),));src.commit();dest=sqlite3.connect(path)
 dest.executescript('CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE tiles(region TEXT,error REAL,x INTEGER,y INTEGER,side REAL,parents TEXT,blob BLOB,auxiliary BLOB,sha256 BLOB,info TEXT,PRIMARY KEY(region,error,x,y)) WITHOUT ROWID;');dest.executemany('INSERT INTO metadata VALUES(?,?)',src.execute('select key,value from metadata'))
 for r,e,x,y,side,parents,blob,sha,info in src.execute('select region,error,x,y,side,parents,blob,sha256,info from tiles'):
  meta=json.loads(info);meta['auxiliaryStorage']='runtime-generated';meta['workingAuxiliaryBytes']=meta.pop('auxiliaryBytes');dest.execute('insert into tiles values(?,?,?,?,?,?,?,?,?,?)',(r,e,x,y,side,parents,blob,None,sha,json.dumps(meta)))
 config=json.loads(dest.execute('select value from metadata where key="config"').fetchone()[0]);config['auxiliaryPolicy']='runtime-generated; SAU1 optional cache';dest.execute('update metadata set value=? where key="config"',(json.dumps(config),));dest.commit();dest.execute('VACUUM')
 if dest.execute('pragma integrity_check').fetchone()[0]!='ok' or dest.execute('select count(*) from tiles').fetchone()[0]!=16383:raise ValueError('配布DBの検証に失敗')
 for a,b in zip(src.execute('select region,error,x,y,sha256,blob from tiles order by region,error,x,y'),dest.execute('select region,error,x,y,sha256,blob from tiles order by region,error,x,y')):
  if a!=b:raise ValueError('配布DBの形状が変わりました')
 report=dict(workingSqliteBytes=(ROOT/'terrain.sqlite3').stat().st_size,distributionSqliteBytes=path.stat().st_size,tiles=16383,geometryBytes=src.execute('select sum(length(blob)) from tiles').fetchone()[0],workingAuxiliaryBytes=src.execute('select sum(length(auxiliary)) from tiles').fetchone()[0],pointBlobsUnchanged=True);(ROOT/'distribution.json').write_text(json.dumps(report,indent=2),encoding='utf-8');src.close();dest.close()
if __name__=='__main__':export()
