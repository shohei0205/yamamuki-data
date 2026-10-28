"""独立生成と親XY保持を、点数増加・共有保存・復元込みで比べる。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
from pathlib import Path
import json,sqlite3,time,statistics,shutil
import numpy as np
from compare_sibson_delta import source,ERRORS
from mls_quant_codec import decode_quant
from sibson_delta_codec import checksum,create_database,load_level
from sibson_shared_codec import encode_shared,decode_shared,load_shared
from sibson_surface import SibsonSurface
ROOT=Path('build/sibson-shared-comparison-v1')
NESTED=Path('build/sibson-nested-v1')
VIEW=Path('build/mls-generated-preview/dem-natural-shared')

def main():
 ROOT.mkdir(exist_ok=True);VIEW.mkdir(exist_ok=True)
 if any((ROOT/(mode+'.sqlite3')).exists() for mode in ['original','nested','shared']):raise ValueError('既存比較は上書きしません')
 for region in ['fuji','yari','tsukuba']:
  for error in ERRORS:
   if not (NESTED/region/f'{error:g}-natural-nested{error:g}'/'info.json').exists():raise ValueError('全品質の生成完了後に実行してください')
 shutil.copyfile('build/sibson-delta-comparison-v1/independent.sqlite3',ROOT/'original.sqlite3')
 dbs={'original':sqlite3.connect(ROOT/'original.sqlite3'),'nested':create_database(ROOT/'nested.sqlite3'),'shared':create_database(ROOT/'shared.sqlite3')}
 dbs['shared'].execute("UPDATE metadata SET value='Sibson NSP1 shared XY experiment v1' WHERE key='format'")
 dbs['shared'].execute("UPDATE metadata SET value='parent canonical vertices plus Z corrections and added XYZ; no parent surface evaluation' WHERE key='predictor'")
 rows=[]
 for region in ['fuji','yari','tsukuba']:
  parent=None;parent_error=None
  for error in ERRORS:
   folder=NESTED/region/f'{error:g}-natural-nested{error:g}';blob=(folder/'model.mlq1').read_bytes();p=decode_quant(blob)
   info=json.loads((folder/'info.json').read_text());old_blob=(source(region,error)/'model.mlq1').read_bytes();old_p=decode_quant(old_blob);old_info=json.loads((source(region,error)/'info.json').read_text())
   encoded=blob if parent is None else encode_shared(p,parent,parent_error)
   if parent is not None:np.testing.assert_array_equal(decode_shared(encoded,parent,parent_error),p)
   for mode in ['nested','shared']:
    dbs[mode].execute('INSERT INTO levels VALUES(?,?,?,?,?)',(region,error,parent_error if mode=='shared' and parent is not None else None,encoded if mode=='shared' else blob,checksum(p)));dbs[mode].commit()
   row=dict(region=region,error=error,originalControls=len(old_p),nestedControls=len(p),addedControls=len(p)-(len(parent) if parent is not None else 0),
    originalBytes=len(old_blob),nestedIndependentBytes=len(blob),sharedBytes=len(encoded),parentError=parent_error,
    originalMaxError=old_info['measuredMaxError'],originalRmse=old_info['measuredRmse'],nestedRmse=info['measuredRmse'],measuredMaxError=info['measuredMaxError'],sigmaMeters=info['sigmaMeters'],extraCompressionError=0.,timings={})
   query=np.random.default_rng(811).uniform(-15999,15999,(8192,2));truth=SibsonSurface(p)(query)
   for mode,db in dbs.items():
    metrics=[]
    for _ in range(3):
     restored,model,stats=load_shared(db,region,error) if mode=='shared' else load_level(db,region,error)
     np.testing.assert_array_equal(restored,old_p if mode=='original' else p)
     if mode!='original':np.testing.assert_array_equal(model(query),truth)
     metrics.append(stats)
    row['timings'][mode]=dict(coldSeconds=statistics.median(m['seconds'] for m in metrics),requiredBytes=metrics[0]['bytes'],requiredLevels=metrics[0]['levels'],
     modelBuildSeconds=statistics.median(m['modelBuildSeconds'] for m in metrics))
   rows.append(row);parent=p;parent_error=error
   print(json.dumps(row),flush=True)
   (ROOT/'progress.json').write_text(json.dumps(dict(stage='measuring',completed=len(rows),region=region,error=error)),encoding='utf-8')
 totals={}
 for mode,db in dbs.items():
  totals[mode]=dict(blobBytes=db.execute('SELECT sum(length(blob)) FROM levels').fetchone()[0],sqliteBytes=(ROOT/(mode+'.sqlite3')).stat().st_size);db.close()
 report=dict(format='Sibson NSP1 shared XY experiment v1',errors=ERRORS,allSharedVerticesBitExact=True,extraCompressionError=0.,
  originalPointSum=sum(r['originalControls'] for r in rows),nestedPointSum=sum(r['nestedControls'] for r in rows),totals=totals,rows=rows,
  timingEnvironment='Python and native C on current Windows; median of three; no model cache; OS file cache uncontrolled')
 for path in [ROOT/'report.json',VIEW/'report.json']:path.write_text(json.dumps(report,indent=2),encoding='utf-8')
 for region in ['fuji','yari','tsukuba']:(VIEW/(region+'.json')).write_text(json.dumps(dict(totals=totals,rows=[r for r in rows if r['region']==region])),encoding='utf-8')
 (VIEW/'index.html').write_text(Path('tools/terrain/sibson_shared_report.html').read_text(encoding='utf-8'),encoding='utf-8',newline='\n')
 (ROOT/'progress.json').write_text(json.dumps(dict(stage='complete',completed=len(rows))),encoding='utf-8')

if __name__=='__main__':main()
