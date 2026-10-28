"""全64アンカー配置を集計し、参照段数別の最小容量案を実測する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
from pathlib import Path
import itertools,json,sqlite3,statistics,time
import numpy as np
from sibson_delta_codec import create_database
from sibson_shared_codec import load_shared
from mls_quant_codec import decode_quant
from compare_sibson_shared import NESTED,ERRORS
ROOT=Path('build/sibson-anchor-comparison-v1')
SOURCE=Path('build/sibson-shared-comparison-v1')
VIEW=Path('build/mls-generated-preview/dem-natural-anchors')
REGIONS=['fuji','yari','tsukuba']

def enumerate_plans(full,shared):
 plans=[]
 for bits in itertools.product([False,True],repeat=len(ERRORS)-1):
  anchors={ERRORS[0]}|{e for e,b in zip(ERRORS[1:],bits) if b};rows=[]
  for region in REGIONS:
   for i,error in enumerate(ERRORS):
    levels=[error];j=i
    while ERRORS[j] not in anchors:j-=1;levels.append(ERRORS[j])
    size=sum(len(full[(region,e)][1]) if e in anchors else len(shared[(region,e)][1]) for e in levels)
    rows.append(dict(region=region,error=error,requiredLevels=levels,requiredBytes=size,blobBytes=len(full[(region,error)][1]) if error in anchors else len(shared[(region,error)][1])))
  plans.append(dict(anchors=sorted(anchors,reverse=True),blobBytes=sum(r['blobBytes'] for r in rows),maxReferences=max(len(r['requiredLevels']) for r in rows),
    meanRequiredBytes=statistics.mean(r['requiredBytes'] for r in rows),fineRequiredBytes=sum(r['requiredBytes'] for r in rows if r['error']==12.5),rows=rows))
 return plans

def main():
 ROOT.mkdir(exist_ok=True);VIEW.mkdir(exist_ok=True)
 if list(ROOT.glob('*.sqlite3')):raise ValueError('既存結果は上書きしません')
 def read(name):
  with sqlite3.connect((SOURCE/(name+'.sqlite3')).resolve().as_uri()+'?mode=ro',uri=True) as db:
   return {(r,e):(pe,b,h) for r,e,pe,b,h in db.execute('SELECT region,error,parent_error,blob,sha256 FROM levels')}
 full,shared=read('nested'),read('shared');plans=enumerate_plans(full,shared)
 selected=[('independent',next(p for p in plans if len(p['anchors'])==len(ERRORS))),('chain',next(p for p in plans if len(p['anchors'])==1))]
 for maximum in [4,3,2]:selected.append(('depth'+str(maximum),min((p for p in plans if p['maxReferences']<=maximum),key=lambda p:(p['blobBytes'],p['anchors']))))
 results=[]
 for name,plan in selected:
  path=ROOT/(name+'.sqlite3');db=create_database(path);anchors=set(plan['anchors'])
  db.execute("UPDATE metadata SET value='Sibson NSP1 independent anchor experiment v1' WHERE key='format'")
  db.execute("UPDATE metadata SET value='parent canonical vertices plus Z corrections and added XYZ; no parent surface evaluation' WHERE key='predictor'")
  db.execute("INSERT INTO metadata VALUES('anchors',?)",(json.dumps(plan['anchors']),))
  for key in full:
   pe,blob,digest=full[key] if key[1] in anchors else shared[key]
   db.execute('INSERT INTO levels VALUES(?,?,?,?,?)',(key[0],key[1],pe,blob,digest))
  db.commit();rows=[]
  for row in plan['rows']:
   expected=decode_quant((NESTED/row['region']/f"{row['error']:g}-natural-nested{row['error']:g}"/'model.mlq1').read_bytes())
   timings=[];build_times=[]
   for _ in range(3):
    points,model,stats=load_shared(db,row['region'],row['error']);np.testing.assert_array_equal(points,expected)
    if stats['levels']!=row['requiredLevels'] or stats['bytes']!=row['requiredBytes']:raise ValueError('参照容量が集計と一致しません')
    timings.append(stats['seconds']);build_times.append(stats['modelBuildSeconds'])
   rows.append(dict(row,coldSeconds=statistics.median(timings),modelBuildSeconds=statistics.median(build_times),verticesBitExact=True))
  db.close()
  results.append(dict(id=name,**{k:v for k,v in plan.items() if k!='rows'},sqliteBytes=path.stat().st_size,
   fineSecondsSum=sum(r['coldSeconds'] for r in rows if r['error']==12.5),meanColdSeconds=statistics.mean(r['coldSeconds'] for r in rows),rows=rows))
  print(json.dumps(dict(strategy=name,anchors=plan['anchors'],blobBytes=plan['blobBytes'],maxReferences=plan['maxReferences'])),flush=True)
  (ROOT/'progress.json').write_text(json.dumps(dict(stage='measuring',completed=len(results))),encoding='utf-8')
 report=dict(enumeratedPlacements=len(plans),baseline='same nested XYZ independently stored at every quality',allVerticesBitExact=True,extraCompressionError=0,
  timingRepeats=3,timingEnvironment='Python and native C on current Windows; no model cache; OS file cache uncontrolled',strategies=results,
  placements=[{k:v for k,v in p.items() if k!='rows'} for p in sorted(plans,key=lambda p:p['blobBytes'])])
 for p in [ROOT/'report.json',VIEW/'report.json']:p.write_text(json.dumps(report,indent=2),encoding='utf-8')
 (VIEW/'index.html').write_text(Path('tools/terrain/sibson_anchor_report.html').read_text(encoding='utf-8'),encoding='utf-8',newline='\n')
 (ROOT/'progress.json').write_text(json.dumps(dict(stage='complete',placements=len(plans),strategies=len(results))),encoding='utf-8')

if __name__=='__main__':main()
