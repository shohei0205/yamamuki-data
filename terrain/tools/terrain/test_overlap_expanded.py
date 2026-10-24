"""事前補修余白を未知のレベル組み合わせ・サイズで検証し、DEM誤差を測る。"""
import json,sqlite3,time
from functools import lru_cache
import numpy as np
import overlap_compare as o
from bake_overlap_halo import tile,execute
from test_overlap_compare import check
ERRORS=[12.5,25,50,100,200,400,800]
RECTS={
 'dual-x':o.bounds('dual',0),'dual-y':o.bounds('dual',1),
 'unequal':[[0,0,4000,16000],[4000,0,16000,16000]],
 'quad':o.bounds('quad'),'tee':o.bounds('tee'),'partial':o.bounds('partial'),
 'unequal-tee':[[0,0,4000,16000],[4000,0,16000,4000],[4000,4000,16000,16000]]}
def raster(mesh):
 axis=np.arange(0,16001,40);z=np.full((len(axis),len(axis)),np.nan);p=np.array(mesh['points'])
 for f in mesh['faces']:
  a,b,c=p[f];lo=np.maximum(0,np.ceil(np.minimum(np.minimum(a[:2],b[:2]),c[:2])/40).astype(int));hi=np.minimum(400,np.floor(np.maximum(np.maximum(a[:2],b[:2]),c[:2])/40).astype(int))
  if np.any(lo>hi):continue
  x,y=np.meshgrid(axis[lo[0]:hi[0]+1],axis[lo[1]:hi[1]+1]);den=(b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])
  u=((x-a[0])*(c[1]-a[1])-(y-a[1])*(c[0]-a[0]))/den;v=((b[0]-a[0])*(y-a[1])-(b[1]-a[1])*(x-a[0]))/den
  inside=(u>=-1e-9)&(v>=-1e-9)&(u+v<=1+1e-9);view=z[lo[1]:hi[1]+1,lo[0]:hi[0]+1];values=a[2]+u*(b[2]-a[2])+v*(c[2]-a[2]);overlap=inside&np.isfinite(view)
  assert not np.any(overlap) or np.max(np.abs(view[overlap]-values[overlap]))<1e-5
  view[inside]=values[inside]
 assert np.isfinite(z).all(),'標高評価点の欠損'
 return z

def run():
 start=time.perf_counter();rows=[];heights=[]
 plane=dict(points=[[0,0,0],[16000,0,160],[16000,16000,480],[0,16000,320]],faces=[[0,1,2],[0,2,3]])
 zz=raster(plane);assert abs(zz[200,200]-240)<1e-8
 for region in ['fuji','yari','tsukuba']:
  with sqlite3.connect(o.OUT/'tiles.sqlite3') as db:meshes={e:json.loads(raw) for e,raw in db.execute('select error,body from refined_meshes_v1 where region=? and width=2000',(region,))}
  @lru_cache(None)
  def get(e,rect):return tile(meshes[e],region,e,list(rect),2000,[])
  configs=[]
  for label in ['dual-x','dual-y','unequal']:
   for a in ERRORS:
    for b in ERRORS:configs.append((label,[a,b]))
  for label in ['quad','tee','partial','unequal-tee']:
   n=len(RECTS[label])
   for i in range(len(ERRORS)):configs.append((label,[ERRORS[(i+k)%len(ERRORS)] for k in range(n)]))
  for label,errors in configs:
   ts=[get(e,tuple(b)) for e,b in zip(errors,RECTS[label])];row=dict(region=region,layout=label,errors=errors)
   try:
    m=execute(dict(tiles=ts,width=2000,local=True,partial=False));check(m,ts)
    row.update(m['stats'],topologyValid=True)
    if len(set(errors))==1:assert m['stats']['sameMeshFastPath'] and m['stats']['triangulationVertices']==0
   except Exception as exc:row.update(topologyValid=False,error=str(exc))
   rows.append(row)
  # 40m間隔で元の10m格子を評価。接合後と事前補修前の方式を同条件で測る。
  dem=np.fromfile(o.ROOT/f'build/decimation-compare-20261023/{region}.f64',dtype='<f8').reshape(1601,1601)[::4,::4]
  with sqlite3.connect(o.SOURCE.as_uri()+'?mode=ro',uri=True) as db:sources={e:json.loads(raw) for e,raw in db.execute("select error,preview from meshes where region=? and method='decimated-boundary'",(region,))}
  for e in ERRORS:
   before=raster(sources[e]);after=raster(meshes[e]);diff=after-before
   heights.append(dict(region=region,kind='whole-level',error=e,beforeMax=float(np.max(np.abs(before-dem))),afterMax=float(np.max(np.abs(after-dem))),beforeRmse=float(np.sqrt(np.mean((before-dem)**2))),afterRmse=float(np.sqrt(np.mean((after-dem)**2))),changedMax=float(np.max(np.abs(diff)))))
  for label,errors in [('dual-x',[50,100]),('quad',[25,50,100,200]),('tee',[200,50,100]),('partial',[200,50,400,100])]:
   ts=[get(e,tuple(b)) for e,b in zip(errors,RECTS[label])];after=raster(execute(dict(tiles=ts,width=2000,local=True,partial=False)))
   original=[o.prepare_tile(region,e,b,2000)[0] for e,b in zip(errors,RECTS[label])];before=raster(execute(dict(tiles=original,width=2000,local=True,partial=False)))
   allowance=np.zeros(dem.shape)
   x,y=np.meshgrid(np.arange(0,16001,40),np.arange(0,16001,40))
   for e,b in zip(errors,RECTS[label]):allowance=np.maximum(allowance,np.where((x>=b[0])&(x<=b[2])&(y>=b[1])&(y<=b[3]),e,0))
   heights.append(dict(region=region,kind='joined',layout=label,errors=errors,beforeMax=float(np.max(np.abs(before-dem))),afterMax=float(np.max(np.abs(after-dem))),beforeRmse=float(np.sqrt(np.mean((before-dem)**2))),afterRmse=float(np.sqrt(np.mean((after-dem)**2))),beforeErrorRatio=float(np.max(np.abs(before-dem)/allowance)),afterErrorRatio=float(np.max(np.abs(after-dem)/allowance))))
  print(region,'接合',len(configs),'標高',len([h for h in heights if h['region']==region]),'完了',flush=True)
  (o.OUT/'halo-refined-expanded.json').write_text(json.dumps(dict(rows=rows,heights=heights,seconds=time.perf_counter()-start),ensure_ascii=False,indent=2),encoding='utf-8')
 print('接合',len(rows),'成功',sum(r['topologyValid'] for r in rows),'角度合格',sum(r.get('qualitySatisfied',False) for r in rows),flush=True)
if __name__=='__main__':run()
