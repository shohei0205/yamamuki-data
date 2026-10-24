import unittest,collections,sys,json,sqlite3
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parent))
from seam_compare import build

def validate(mesh,axis):
 p=mesh['points'];edges=collections.Counter();area=0
 for f in mesh['faces']:
  a,b,c=[p[i] for i in f];d=(b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0]);assert d>0;area+=d
  for i in range(3):edges[tuple(sorted((f[i],f[(i+1)%3])))]+=1
 assert abs(area-2*16000**2)<.1,(area,'area')
 for (a,b),count in edges.items():
  assert count in (1,2)
  if count==1:assert any(abs(p[a][k]-p[b][k])<1e-6 and (abs(p[a][k])<1e-6 or abs(p[a][k]-16000)<1e-6) for k in (0,1)),('unconnected',p[a],p[b])
 for patch in mesh.get('patches',[mesh.get('patch')]):
  if patch is None:continue
  assert all(mesh['roles'][i]==2 for i in patch['faceIds'])
  for f,fi in zip(patch['faces'],patch['faceIds']):
   assert [patch['points'][v] for v in f]==[p[v] for v in mesh['faces'][fi]]
class SeamTest(unittest.TestCase):
 def test_crossing_diagonals_and_both_axes(self):
  fine=dict(points=[[0,0,0],[16000,0,0],[16000,16000,0],[0,16000,0],[8000,8000,100]],faces=[[0,1,4],[1,2,4],[2,3,4],[3,0,4]])
  coarse=dict(points=fine['points'][:4],faces=[[0,1,2],[0,2,3]])
  for axis in (0,1):
   m=build(fine,coarse,axis);validate(m,axis);self.assertTrue(m['patch']['extraVertexIds']);self.assertEqual(m['patch']['owner'],'fine')
 def test_same_level(self):
  m=dict(points=[[0,0,0],[16000,0,0],[16000,16000,0],[0,16000,0]],faces=[[0,1,2],[0,2,3]])
  validate(build(m,m),0)
def validate_real_data():
 from seam_compare import OUT,make
 count=0
 with sqlite3.connect(OUT/'comparison.sqlite3') as db:
  for region in ['fuji','yari','tsukuba']:
   meshes={e:json.loads(body) for e,body in db.execute("select error,preview from meshes where region=? and method='decimated-boundary'",(region,))}
   for fine,coarse in [(12.5,25),(25,50),(50,100),(100,200),(200,400),(400,800),(50,800)]:
    for axis in (0,1):
     validate(build(meshes[fine],meshes[coarse],axis),axis);count+=1
   print(region,'接続の検証に成功',flush=True)
 print(count,'ケースの面積・面の向き・辺の接続・接合データ復元を確認')
def validate_rust_data():
 from seam_compare import make_rust
 count=0
 for region in ['fuji','yari','tsukuba']:
  for fine,coarse in [(12.5,25),(25,50),(50,100),(100,200),(200,400),(400,800),(50,800)]:
   for axis in (0,1):
    validate(make_rust(region,[fine,coarse],axis),axis);count+=1
  for errors in [[25,50,100,200],[200,100,50,25],[800,400,200,100],[50,50,50,50]]:
   m=make_rust(region,errors,quad=True);validate(m,0);count+=1
   if len(set(errors))==1:assert not m['patches']
  print(region,'Rust接合の検証に成功',flush=True)
 print(count,'ケースの面積・面の向き・辺の接続・接合データ復元を確認')
def validate_shared_data():
 from seam_compare import make_rust,OUT
 rows=[]
 with sqlite3.connect(OUT/'comparison.sqlite3') as db:refs={r:json.loads(db.execute("select preview from meshes where region=? and method='decimated-boundary' and error=12.5",(r,)).fetchone()[0])['points'] for r in ['fuji','yari','tsukuba']}
 for region in refs:
  for lo,hi in [(12.5,25),(25,50),(50,100),(100,200),(200,400),(400,800),(50,800)]:
   configs=[([lo,hi],axis,False) for axis in (0,1)]+[([hi,lo],axis,False) for axis in (0,1)]+[([lo,hi,hi,lo],0,True),([hi,lo,lo,hi],0,True)]
   for errors,axis,quad in configs:
    m=make_rust(region,errors,axis,quad=quad,shared=True);validate(m,axis)
    assert all(all(abs(a-b)<1e-8 for a,b in zip(p,refs[region][i])) for p,i in zip(m['points'],m['sourceIds']))
    assert len(m['points'])==len(m['sourceIds'])==len(set(m['sourceIds']))
    assert m['stats']['minimumAngle']>=10-1e-6 and m['stats']['sharpTriangles']==0
    rows.append(dict(region=region,errors=errors,axis=axis,quad=quad,**m['stats']))
   print(region,lo,hi,'OK',flush=True)
 print('共有頂点方式',len(rows),'ケースに成功。詳細形状への復帰:',sum(r['fullReferenceFallback'] for r in rows))
if __name__=='__main__':
 if '--shared' in sys.argv:validate_shared_data()
 elif '--rust' in sys.argv:validate_rust_data()
 elif '--real' in sys.argv:validate_real_data()
 else:unittest.main()
