"""余白付きタイルの接続、本体面の保持、保存タイルだけでの復元を検証する。"""
import json,sqlite3,sys
from pathlib import Path
from unittest import TestCase,main
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parent))
import overlap_compare as overlap
from test_seam_compare import validate

def check(mesh,tiles):
 validate(mesh,0)
 def point(p):return (round(p[0],6),round(p[1],6),round(p[2],6))
 faces={tuple(sorted(point(mesh['points'][v]) for v in f)) for f in mesh['faces']}
 loaded={point(p) for tile in tiles for p in tile['points']+tile['support']}
 assert all(point(p) in loaded for p in mesh['points'])
 for item in mesh['preservedCore']:
  tile=tiles[item['tile']];f=tile['faces'][item['face']];assert tuple(sorted(point(tile['points'][v]) for v in f)) in faces
 assert mesh['stats']['coreFaces']==len(mesh['preservedCore'])==mesh['stats']['retainedCoreFaces']
class LayoutTest(TestCase):
 def test_changed_faces_compare_coordinates(self):
  source=dict(points=[[0,0,0],[1,0,0],[1,1,0],[0,1,0]],faces=[[0,1,2],[0,2,3]])
  same=dict(points=list(reversed(source['points'])),faces=[[1,2,3],[0,1,3]],stats={})
  overlap.mark_changed_faces(same,[source]);self.assertEqual(same['changedFaces'],[False,False])
  changed=dict(points=source['points'],faces=[[0,1,3],[1,2,3]],stats={})
  overlap.mark_changed_faces(changed,[source]);self.assertEqual(changed['changedFaces'],[True,True]);self.assertEqual(changed['stats']['changedTriangles'],2)
 def test_all_unloaded_does_not_read_or_run(self):
  with patch.object(overlap,'prepare_tile') as prepare,patch.object(overlap.subprocess,'run') as execute:
   m=overlap.make('fuji',[None]*4,'quad',width=2000)
  prepare.assert_not_called();execute.assert_not_called();self.assertEqual(m['points'],[]);self.assertEqual(m['activeBounds'],[]);self.assertEqual(m['stats']['loadedBytes'],0)
 def test_layouts_cover_rectangle(self):
  for layout,n in [('dual',2),('quad',4),('tee',3),('partial',4)]:
   rects=overlap.bounds(layout);self.assertEqual(len(rects),n);self.assertEqual(sum((u-x)*(v-y) for x,y,u,v in rects),16000**2)
   for i,a in enumerate(rects):
    for b in rects[i+1:]:self.assertTrue(min(a[2],b[2])<=max(a[0],b[0]) or min(a[3],b[3])<=max(a[1],b[1]))

def real():
 rows=[]
 for region in ['fuji','yari','tsukuba']:
  for layout,errors in [('dual',[50,100]),('quad',[25,50,100,200]),('tee',[200,50,100]),('partial',[200,50,400,100])]:
   for width in [500,1000,2000,4000]:
    tiles=[overlap.prepare_tile(region,e,b,width)[0] for e,b in zip(errors,overlap.bounds(layout))];mesh=overlap.make(region,errors,layout,width=width);check(mesh,tiles)
    if width>=2000:assert mesh['stats']['qualitySatisfied']
    rows.append(dict(region=region,layout=layout,**mesh['stats']))
   print(region,layout,'本体面と接続を確認',flush=True)
 # 全タイルが保存された後は、元メッシュDBがなくてもマージできる。
 previous=overlap.SOURCE
 try:
  overlap.SOURCE=overlap.OUT/'source-is-unavailable.sqlite3'
  mesh=overlap.make('fuji',[200,50,400,100],'partial',width=2000);assert mesh['stats']['qualitySatisfied']
 finally:overlap.SOURCE=previous
 (overlap.OUT/'audit.json').write_text(json.dumps(rows,indent=2),encoding='utf-8')
 print(len(rows),'ケースで本体・接続・読み込み頂点の範囲を検証。保存タイルだけの実行にも成功。')
def unloaded():
 count=0
 for layout,n in [('dual',2),('quad',4),('tee',3),('partial',4)]:
  for mask in range(1<<n):
   errors=[([200,50,400,100][i] if mask&(1<<i) else None) for i in range(n)]
   for local in (False,True):
    with patch.object(overlap,'prepare_tile',wraps=overlap.prepare_tile) as spy:
     m=overlap.make('fuji',errors,layout,width=2000,local=local)
    active=[i for i,e in enumerate(errors) if e is not None];assert spy.call_count==len(active);assert all(call.args[1] is not None for call in spy.call_args_list)
    assert m['tileSlots']==active and m['activeBounds']==[overlap.bounds(layout)[i] for i in active] and m['stats']['tileCount']==len(active)
    if not active:assert not m['faces'] and m['stats']['loadedBytes']==0
    else:
     p=m['points'];edges={}
     for f in m['faces']:
      a,b,c=[p[v] for v in f];assert (b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])>0
      for k in range(3):
       edge=tuple(sorted((f[k],f[(k+1)%3])));edges[edge]=edges.get(edge,0)+1
     assert all(v in (1,2) for v in edges.values())
     assert len(p)-len(edges)+len(m['faces'])==1
     assert m['stats']['coreFaces']==m['stats']['retainedCoreFaces']
    count+=1
  print(layout,'未読込の全組み合わせに成功',flush=True)
 print(count,'ケースで取得対象と形状を確認')
def same_level():
 count=0
 for region in ['fuji','yari','tsukuba']:
  for layout,n in [('dual',2),('quad',4),('tee',3),('partial',4)]:
   errors=[50]*n
   tiles=[overlap.prepare_tile(region,e,b,2000)[0] for e,b in zip(errors,overlap.bounds(layout))]
   m=overlap.make(region,errors,layout,width=2000);check(m,tiles)
   assert m['stats']['sameMeshFastPath'] and m['stats']['triangulationVertices']==0 and m['stats']['patchTriangles']==0
   def faces(points,items):return {tuple(sorted(tuple(round(v,6) for v in p) for p in (points[i] for i in f))) for f in items}
   expected=set().union(*(faces(t['points'],t['faces']) for t in tiles))
   assert faces(m['points'],m['faces'])==expected
   count+=1
  for layout,errors in [('quad',[50,50,100,100]),('tee',[100,50,50]),('partial',[100,50,100,100])]:
   tiles=[overlap.prepare_tile(region,e,b,2000)[0] for e,b in zip(errors,overlap.bounds(layout))]
   m=overlap.make(region,errors,layout,width=2000);check(m,tiles)
   assert not m['stats']['sameMeshFastPath'] and m['stats']['sameLevelSkippedSegments']>0
   count+=1
  print(region,'同レベルと混在接合を確認',flush=True)
 print(count,'ケースで同レベル省略と形状保持を確認')
if __name__=='__main__':
 if '--same-level' in sys.argv:same_level()
 elif '--unloaded' in sys.argv:unloaded()
 elif '--real' in sys.argv:real()
 else:main()
