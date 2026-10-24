"""余白事前補修の接合品質、同レベル省略、容量を確認する。"""
import json
import overlap_compare as o
from bake_overlap_halo import CONFIGS
from test_overlap_compare import check

def run():
 rows=[]
 for region in ['fuji','yari','tsukuba']:
  for layout,errors in CONFIGS:
   tiles=[o.prepare_refined_tile(region,e,b,2000)[0] for e,b in zip(errors,o.bounds(layout))]
   assert all(not t['support'] for t in tiles)
   m=o.make(region,errors,layout,width=2000,refined=True);check(m,tiles)
   if max(errors)<800:assert m['stats']['qualitySatisfied']
   original=[o.prepare_tile(region,e,b,2000)[0] for e,b in zip(errors,o.bounds(layout))]
   rows.append(dict(region=region,layout=layout,errors=errors,**m['stats'],originalBytes=sum(len(json.dumps(t,separators=(',',':')).encode()) for t in original),noSupportBytes=sum(len(json.dumps(dict(t,support=[]),separators=(',',':')).encode()) for t in original)))
  for error in [12.5,25,50,100,200,400,800]:
   tiles=[o.prepare_refined_tile(region,error,b,2000)[0] for b in o.bounds('quad')]
   m=o.make(region,[error]*4,'quad',width=2000,refined=True);check(m,tiles)
   assert m['stats']['sameMeshFastPath'] and m['stats']['triangulationVertices']==0
  print(region,'通常接合・同レベル・極端な差の診断を確認',flush=True)
 (o.OUT/'halo-refined-comparison.json').write_text(json.dumps(rows,ensure_ascii=False,indent=2),encoding='utf-8')
 for region in ['fuji','yari','tsukuba']:
  r=next(r for r in rows if r['region']==region and r['layout']=='quad' and max(r['errors'])<800)
  print(region,'通常4領域',r['minimumAngle'],r['sharpTriangles'],'bytes',r['originalBytes'],r['noSupportBytes'],r['loadedBytes'])
if __name__=='__main__':run()
