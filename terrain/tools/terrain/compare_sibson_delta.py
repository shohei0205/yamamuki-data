"""自然近傍の独立・全段差分・独立アンカー併用を実測比較する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
from pathlib import Path
import json,sqlite3,time,statistics
import numpy as np
from mls_quant_codec import decode_quant
from sibson_surface import SibsonSurface
from sibson_delta_codec import encode_delta,decode_delta,load_level,create_database,checksum
ROOT=Path('build/sibson-delta-comparison-v1')
VIEW=Path('build/mls-generated-preview/dem-natural-compression')
ERRORS=[800,400,200,100,50,25,12.5]
ANCHORS={800,200,50,12.5}

def source(region,error):
 root='sibson-error-fine-v1' if error<50 else 'sibson-error-driven-v1' if error==50 else 'sibson-error-levels-v1'
 return Path('build')/root/region/f'{error:g}-natural-error{error:g}'

def main():
 ROOT.mkdir(exist_ok=True);VIEW.mkdir(exist_ok=True)
 if any((ROOT/(mode+'.sqlite3')).exists() for mode in ['independent','chain','anchors']):raise ValueError('既存比較結果は上書きしません')
 dbs={mode:create_database(ROOT/(mode+'.sqlite3')) for mode in ['independent','chain','anchors']}
 rows=[]
 for region in ['fuji','yari','tsukuba']:
  parent=None;parent_error=None
  for error in ERRORS:
   folder=source(region,error);blob=(folder/'model.mlq1').read_bytes();p=decode_quant(blob)
   info=json.loads((folder/'info.json').read_text());delta=blob if parent is None else encode_delta(p,parent,parent_error)
   if parent is not None:np.testing.assert_array_equal(decode_delta(delta,parent,parent_error),p)
   digest=checksum(p)
   for mode,db in dbs.items():
    use_delta=parent is not None and (mode=='chain' or mode=='anchors' and error not in ANCHORS)
    db.execute('INSERT INTO levels VALUES(?,?,?,?,?)',(region,error,parent_error if use_delta else None,delta if use_delta else blob,digest));db.commit()
   row=dict(region=region,error=error,controls=len(p),independentBytes=len(blob),deltaBytes=len(delta),
            parentError=parent_error,verifiedRawMaxError=info['measuredMaxError'],sigmaMeters=info['sigmaMeters'],
            vertexMaxDifference=0.,surfaceMaxDifference=0.,timings={})
   query=np.random.default_rng(883).uniform(-15999,15999,(8192,2))
   original_model=SibsonSurface(p);truth=original_model(query)
   for mode,db in dbs.items():
    timings=[];stats_list=[]
    for repeat in range(3):
     restored,model,stats=load_level(db,region,error)
     np.testing.assert_array_equal(restored,p)
     np.testing.assert_array_equal(model(query),truth)
     timings.append(stats['seconds']);stats_list.append(stats)
    row['timings'][mode]=dict(coldSeconds=statistics.median(timings),
     requiredBytes=stats_list[0]['bytes'],requiredLevels=stats_list[0]['levels'],
     blobDecodeSeconds=statistics.median(s['blobDecodeSeconds'] for s in stats_list),
     modelBuildSeconds=statistics.median(s['modelBuildSeconds'] for s in stats_list),
     predictionSeconds=statistics.median(s['predictionSeconds'] for s in stats_list))
   if parent is not None:
    timings=[]
    for repeat in range(3):
     t=time.perf_counter();restored=decode_delta(delta,parent,parent_error);assert checksum(restored)==digest;model=SibsonSurface(restored);timings.append(time.perf_counter()-t)
    row['warmParentSeconds']=statistics.median(timings)
   else:row['warmParentSeconds']=row['timings']['independent']['coldSeconds']
   rows.append(row);parent=original_model;parent_error=error
   (ROOT/'progress.json').write_text(json.dumps(dict(stage='measuring',region=region,error=error,completed=len(rows))),encoding='utf-8')
   print(json.dumps(dict(region=region,error=error,independentBytes=len(blob),deltaBytes=len(delta))),flush=True)
 for db in dbs.values():db.close()
 totals={}
 for mode in dbs:
  with sqlite3.connect(ROOT/(mode+'.sqlite3')) as db:
   totals[mode]=dict(blobBytes=db.execute('SELECT sum(length(blob)) FROM levels').fetchone()[0],sqliteBytes=(ROOT/(mode+'.sqlite3')).stat().st_size)
 report=dict(format='Sibson SDZ1 experiment v1',quantizationMeters=.125,prediction='rounded-parent-Sibson-height',
  anchors=sorted(ANCHORS,reverse=True),timingRepeats=3,timingEnvironment='Python ctypes native C / current Windows host; no model cache; OS file cache uncontrolled',
  allVerticesBitExact=True,extraHeightError=0.,totals=totals,rows=rows)
 (ROOT/'report.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
 (VIEW/'report.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
 for region in ['fuji','yari','tsukuba']:
  (VIEW/(region+'.json')).write_text(json.dumps(dict(totals=totals,rows=[r for r in rows if r['region']==region])),encoding='utf-8')
 (VIEW/'index.html').write_text(Path('tools/terrain/sibson_compression_report.html').read_text(encoding='utf-8'),encoding='utf-8',newline='\n')
 (ROOT/'progress.json').write_text(json.dumps(dict(stage='complete',completed=len(rows))),encoding='utf-8')

if __name__=='__main__':main()
