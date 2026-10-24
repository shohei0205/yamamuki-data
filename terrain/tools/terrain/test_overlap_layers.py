"""隣接層による余白の切り出しと接合を検証する。"""
import json,unittest
from pathlib import Path
import overlap_compare as o
from overlap_layers import extract,reach
from test_overlap_compare import check
from test_overlap_expanded import RECTS,ERRORS
class LayerTest(unittest.TestCase):
 def test_bbox_overlap_is_not_intersection(self):
  m=dict(meshKey='triangle',points=[[0,0,0],[10,0,0],[0,10,0]],faces=[[0,1,2]])
  self.assertEqual(extract(m,[8,8,9,9],2)['faces'],[])
 def test_layers_grow_by_shared_edges(self):
  ps=[[x,y,0] for y in range(7) for x in range(7)];fs=[]
  for y in range(6):
   for x in range(6):
    a=y*7+x;fs.extend([[a,a+1,a+8],[a,a+8,a+7]])
  m=dict(meshKey='grid',points=ps,faces=fs)
  def face_set(t):return {tuple(sorted(tuple(t['points'][i]) for i in f)) for f in t['faces']}
  seed=face_set(extract(m,[2.2,2.2,3.8,3.8],0));expected=set(seed)
  allfaces=[tuple(sorted(tuple(ps[i]) for i in f)) for f in fs]
  for layer in range(1,3):
   expected|={f for f in allfaces if any(len(set(f)&set(g))==2 for g in expected)}
   t=extract(m,[2.2,2.2,3.8,3.8],layer)
   self.assertEqual(face_set(t),expected);self.assertFalse(t['support']);self.assertEqual(t['bodyFaces'],len(seed));self.assertEqual(t['haloFaces'],len(expected)-len(seed))

def real():
 rows=[]
 for region in ['fuji','yari','tsukuba']:
  configs=[]
  for label,layout,axis in [('dual-x','dual',0),('dual-y','dual',1)]:
   for a in ERRORS:
    for b in ERRORS:configs.append((label,layout,axis,[a,b]))
  for layout in ['quad','tee','partial']:
   n=len(o.bounds(layout))
   for i in range(7):configs.append((layout,layout,0,[ERRORS[(i+k)%7] for k in range(n)]))
  configs += [('standard-quad','quad',0,[25,50,100,200]),('standard-tee','tee',0,[200,50,100]),('standard-partial','partial',0,[200,50,400,100])]
  for label,layout,axis,errors in configs:
   ts=[o.prepare_layer_tile(region,e,b,2)[0] for e,b in zip(errors,o.bounds(layout,axis))]
   row=dict(region=region,layout=label,errors=errors)
   try:
    m=o.make(region,errors,layout,axis=axis,width=2000,candidates=False,refined=True,layers=2);check(m,ts);row.update(m['stats'],topologyValid=True)
    if len(set(errors))==1:assert m['stats']['sameMeshFastPath'] and m['stats']['triangulationVertices']==0
    original=[o.prepare_refined_tile(region,e,b,2000)[1] for e,b in zip(errors,o.bounds(layout,axis))];row['distanceBytes']=sum(original)
   except Exception as exc:row.update(topologyValid=False,error=str(exc))
   rows.append(row)
  print(region,len(configs),'接合完了',flush=True)
  (o.OUT/'halo-layers-audit.json').write_text(json.dumps(rows,ensure_ascii=False,indent=2),encoding='utf-8')
 print('合計',len(rows),'接続',sum(r['topologyValid'] for r in rows),'角度',sum(r.get('qualitySatisfied',False) for r in rows),flush=True)
 for region in ['fuji','yari','tsukuba']:
  r=next(r for r in rows if r['region']==region and r['layout']=='standard-quad');print(region,'2層',r)
if __name__=='__main__':
 import sys
 if '--real' in sys.argv:real()
 else:unittest.main()
