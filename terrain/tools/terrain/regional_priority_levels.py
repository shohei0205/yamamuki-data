"""3地域のL0～L7を基本16,384点＋追加1,024点で比較生成する。"""
import argparse,json,sqlite3,struct,subprocess,tempfile,zlib,math,os
from concurrent.futures import ProcessPoolExecutor,as_completed
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.ndimage import map_coordinates
import national_independent_mesh as national
from relative_mesh_preview import REGIONS,error_limits,ridge_limits
from national_mesh_levels import ERRORS
from curve_mesh_refinement import curve_candidates,mesh_errors
ROOT=national.ROOT
OUTPUT=Path(os.environ.get('TERRAIN_REGIONAL_OUTPUT',str(ROOT/'build/terrain-regional-priority-levels-v2.sqlite3')))
BASE_CAP=16384
ADDED_CAP=1024

def specifications():
 return [dict(level=l,widthMeters=1000*2**l,maxErrorMeters=e,baseMaxVertices=BASE_CAP,additionalMaxVertices=ADDED_CAP) for l,e in enumerate(ERRORS)]

def region_tiles(lon,lat,level):
 w=1000*2**level;x,y=national.project(lon,lat);cx,cy=math.floor(x/w),math.floor(y/w)
 radius=1 if level<=4 else 0
 return [(level,cx+dx,cy+dy) for dx in range(-radius,radius+1) for dy in range(-radius,radius+1)]

def boundary_points(n):
 # 中点から段階的に挿入し、長い辺へ1点ずつ加えるときの再評価を減らす。
 points=[(0,0),(n-1,0),(0,n-1),(n-1,n-1)];intervals=[(0,n-1)]
 while intervals:
  following=[]
  for lo,hi in intervals:
   if hi-lo<=1:continue
   mid=(lo+hi)//2;points.extend([(mid,0),(mid,n-1),(0,mid),(n-1,mid)])
   following.extend([(lo,mid),(mid,hi)])
  intervals=following
 return points

def adaptive_boundary_points(grid,tolerance):
 """各共有辺の1次元最大標高誤差から、両タイル共通の点だけを残す。"""
 n=len(grid);chosen={(0,0),(n-1,0),(0,n-1),(n-1,n-1)}
 for coords in [[(i,0) for i in range(n)],[(i,n-1) for i in range(n)],[(0,j) for j in range(n)],[(n-1,j) for j in range(n)]]:
  values=np.array([grid[j,i] for i,j in coords]);stack=[(0,n-1)]
  while stack:
   a,b=stack.pop()
   if b-a<2:continue
   predicted=values[a]+(values[b]-values[a])*np.arange(1,b-a)/(b-a)
   errors=np.abs(values[a+1:b]-predicted);at=a+1+int(np.argmax(errors))
   if errors.max()>tolerance*(1-1e-10):chosen.add(coords[at]);stack.extend([(a,at),(at,b)])
 # 共有辺の選定自体は方向によらず、挿入は中点順で速くする。
 return [p for p in boundary_points(n) if p in chosen]

def initialize_worker():
 national.initialize(ROOT/'build/terrain-lod.sqlite3',os.environ.get('TERRAIN_TIN_BINARY',str(ROOT/'build/terrain-rust-target/release/terrain-adaptive-tin.exe')))

def generate_tile(key):
 level,x,y=key;width=1000*2**level;error=ERRORS[level];n=min(2049,max(129,2**int(round(math.log2(width/15.625)))+1));spacing=width/(n-1)
 # 地形の重みは物理距離で評価し、細かいタイルでも周辺6kmを含める。
 field_spacing=max(62.5,spacing);pad=math.ceil(6000/field_spacing);core=round(width/field_spacing)+1
 axis=(np.arange(core+2*pad)-pad)*field_spacing;xx,yy=np.meshgrid(x*width+axis,(y+1)*width-axis)
 halo,valid=national.source_samples(xx,yy);del xx,yy
 reference=error_limits(halo,valid,field_spacing,minimum=max(.25,error*.1),maximum=error)
 weight=np.clip(1-ridge_limits(halo,valid,field_spacing,np.zeros_like(halo))/50,0,1)
 field=error*(1-weight)+weight*reference/(1+weight)
 axis=np.arange(n)*spacing;xx,yy=np.meshgrid(x*width+axis,(y+1)*width-axis);grid,mask=national.source_samples(xx,yy);del xx,yy
 grid=np.rint(grid*4)/4
 fy,fx=np.meshgrid(np.arange(n)*spacing/field_spacing+pad,np.arange(n)*spacing/field_spacing+pad,indexing='ij')
 limits=map_coordinates(field,[fy,fx],order=1,mode='nearest');limits=np.maximum(.01,np.rint(limits*100)/100)
 # 境界は基本誤差の半分以内で1次元簡略化し、隣接タイルで共有する。
 seeds=adaptive_boundary_points(grid,error*.5);seed_set=set(seeds)
 with tempfile.TemporaryDirectory(dir=ROOT/'build',prefix='regional-level-') as temp:
  temp=Path(temp);offset=min(0,float(grid.min()));Image.fromarray(np.rint((grid-offset)*4).astype(np.uint16)).save(temp/'height.png');Image.fromarray(np.rint(limits*100).astype(np.uint16)).save(temp/'limits.png')
  (temp/'seeds.json').write_text(json.dumps(seeds),encoding='utf-8')
  cmd=[national.BINARY,'--input',str(temp/'height.png'),'--output',str(temp/'base.json'),'--scale','.25','--offset',str(offset),'--max-error',str(error),'--min-vertices','4','--max-vertices',str(BASE_CAP),'--seed-points',str(temp/'seeds.json'),'--lock-seed-boundary']
  run=subprocess.run(cmd,check=True,capture_output=True,text=True)
  base=json.loads((temp/'base.json').read_text(encoding='utf-8'));bp=np.asarray(base['vertices']);base_count=len(bp);base_ids={(int(p[0]),int(p[1]),float(p[2])) for p in bp}
  (temp/'seeds.json').write_text(json.dumps(bp[:,:2].astype(int).tolist()),encoding='utf-8')
  cmd[cmd.index('--output')+1]=str(temp/'final.json');cmd[cmd.index('--max-vertices')+1]=str(len(bp)+ADDED_CAP);cmd+=['--error-limits',str(temp/'limits.png')]
  subprocess.run(cmd,check=True,capture_output=True,text=True)
  mesh=json.loads((temp/'final.json').read_text(encoding='utf-8'));p=np.asarray(mesh['vertices']);f=np.asarray(mesh['triangles'],np.uint32)[:,[0,2,1]]
  curve_added=0;curve_remaining=0
  if os.environ.get('TERRAIN_CURVE_AWARE')=='1':
   # ヒントと同じ周辺DEMの微分を、標高格子の画素単位へ換算する。
   grid_gx=map_coordinates(np.gradient(halo,field_spacing,axis=1),[fy,fx],order=1)*spacing
   grid_gy=map_coordinates(np.gradient(halo,field_spacing,axis=0),[fy,fx],order=1)*spacing
   curve_budget=min(1024,BASE_CAP-len(bp))
   initial_final={(int(v[0]),int(v[1]),float(v[2])) for v in p};initial_count=len(p)
   curve_cmd=cmd.copy();at=curve_cmd.index('--error-limits');del curve_cmd[at:at+2]
   for iteration in range(8):
    candidates=curve_candidates(p,np.asarray(mesh['triangles']),grid,grid_gx,grid_gy,error,min(128,curve_budget-curve_added))
    if not candidates:break
    coords=[xy for xy,_ in candidates];seed_points=p[:,:2].astype(int).tolist()+[list(xy) for xy in coords]
    (temp/'seeds.json').write_text(json.dumps(seed_points),encoding='utf-8')
    curve_cmd[curve_cmd.index('--max-vertices')+1]=str(initial_count+curve_budget)
    subprocess.run(curve_cmd,check=True,capture_output=True,text=True)
    mesh=json.loads((temp/'final.json').read_text(encoding='utf-8'));p=np.asarray(mesh['vertices']);f=np.asarray(mesh['triangles'],np.uint32)[:,[0,2,1]]
    new_ids={(int(v[0]),int(v[1]),float(v[2])) for v in p}-initial_final
    base_ids.update(new_ids);curve_added=len(new_ids)
    if curve_added>=curve_budget:break
   curve_remaining=len(curve_candidates(p,np.asarray(mesh['triangles']),grid,grid_gx,grid_gy,error,1000000))
   mesh['maxError'],mesh['maxErrorRatio']=mesh_errors(p,np.asarray(mesh['triangles']),grid,limits)
   base_count+=curve_added

 assert base_count<=BASE_CAP and len(p)-base_count<=ADDED_CAP
 assert base_ids.issubset({(int(v[0]),int(v[1]),float(v[2])) for v in p})
 # 各辺の全点が一致することを、検証用の列として保存する。
 edges={name:[[int(i if name in ['north','south'] else j),float(grid[j,i])] for i,j in coords if (i,j) in seed_set] for name,coords in [('north',[(i,0) for i in range(n)]),('south',[(i,n-1) for i in range(n)]),('west',[(0,j) for j in range(n)]),('east',[(n-1,j) for j in range(n)])]}
 world=p.copy();world[:,0]=p[:,0]*spacing;world[:,1]=width-p[:,1]*spacing
 dx=map_coordinates(np.gradient(halo,field_spacing,axis=1),[p[:,1]*spacing/field_spacing+pad,p[:,0]*spacing/field_spacing+pad],order=1)
 dy=map_coordinates(np.gradient(halo,field_spacing,axis=0),[p[:,1]*spacing/field_spacing+pad,p[:,0]*spacing/field_spacing+pad],order=1)
 hints=np.column_stack([dx,dy]);geometry=zlib.compress(struct.pack('<4sHHII',b'YNT2',n,n,len(p),f.size)+world.astype('<f4').tobytes()+f.astype('<u2').tobytes(),9)
 h=zlib.compress(struct.pack('<4sIf',b'YNH1',len(p),8.)+np.rint(np.clip(hints,-8,8)/8*32767).astype('<i2').tobytes(),9);m=zlib.compress(np.packbits(mask.ravel(),bitorder='little').tobytes(),9)
 added=[]
 for i,v in enumerate(p):
  if (int(v[0]),int(v[1]),float(v[2])) not in base_ids:added.append([float(x*width+world[i,0]),float(y*width+world[i,1]),float(v[2])])
 stats=dict(curveAware=os.environ.get('TERRAIN_CURVE_AWARE')=='1',curveAddedVertices=curve_added,curveRemainingCandidates=curve_remaining,boundaryPolicy='adaptive-half-error-v1',boundaryVertices=len(seeds),boundaryErrorMeters=error*.5,level=level,x=x,y=y,vertices=len(p),triangles=len(f),baseVertices=base_count,baseMaxErrorMeters=base['maxError'],baseToleranceReached=base['toleranceReached'],baseMaxVertices=BASE_CAP,addedVertices=len(added),additionalMaxVertices=ADDED_CAP,maxErrorMeters=mesh['maxError'],maxErrorRatio=mesh['maxErrorRatio'],toleranceReached=mesh['maxErrorRatio']<=1+1e-8,sampleSpacingMeters=spacing,bytes=len(geometry)+len(h)+len(m),addedPositions=added,edges=edges)
 return key,geometry,h,m,stats

def generate(workers=2,binary=None):
 if binary:os.environ['TERRAIN_TIN_BINARY']=str(Path(binary).resolve())
 initialize_worker();tasks=sorted({key for _,_,lon,lat in REGIONS for l in range(8) for key in region_tiles(lon,lat,l)})
 OUTPUT.parent.mkdir(exist_ok=True)
 with sqlite3.connect(OUTPUT) as db:
  db.executescript('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE IF NOT EXISTS tiles(level INTEGER,x INTEGER,y INTEGER,geometry BLOB,hints BLOB,mask BLOB,stats TEXT,PRIMARY KEY(level,x,y));')
  expected_curve=os.environ.get('TERRAIN_CURVE_AWARE')=='1'
  for report, in db.execute('SELECT stats FROM tiles'):
   if bool(json.loads(report).get('curveAware',False))!=expected_curve:raise ValueError('生成方式が異なるデータがあります。別の --output を指定してください')
  done={tuple(r) for r in db.execute('SELECT level,x,y FROM tiles')};pending=[k for k in tasks if k not in done]
  print('全',len(tasks),'タイル／残り',len(pending),flush=True)
  with ProcessPoolExecutor(max_workers=workers,initializer=initialize_worker) as pool:
   futures={pool.submit(generate_tile,k):k for k in pending}
   for future in as_completed(futures):
    key,g,h,m,stats=future.result();db.execute('INSERT OR REPLACE INTO tiles VALUES(?,?,?,?,?,?,?)',(*key,g,h,m,json.dumps(stats)));db.commit();done.add(key)
    print(len(done),'/',len(tasks),key,'基本',stats['baseVertices'],'追加',stats['addedVertices'],'誤差',round(stats['maxErrorMeters'],2),flush=True)
  # 固定した共有辺も含む無次元の誤差比で、追加条件の達成を集計する。
  for level,x,y,report in db.execute('SELECT level,x,y,stats FROM tiles').fetchall():
   record=json.loads(report);record['toleranceReached']=record['maxErrorRatio']<=1+1e-8
   db.execute('UPDATE tiles SET stats=? WHERE level=? AND x=? AND y=?',(json.dumps(record),level,x,y))
  db.commit()
  regions=[]
  for id,name,lon,lat in REGIONS:
   levels=[]
   for l in range(8):
    keys=region_tiles(lon,lat,l);rows=[json.loads(db.execute('SELECT stats FROM tiles WHERE level=? AND x=? AND y=?',k).fetchone()[0]) for k in keys]
    levels.append(dict(curveAddedVertices=sum(r.get('curveAddedVertices',0) for r in rows),curveUnmet=sum(r.get('curveRemainingCandidates',0)>0 for r in rows),level=l,widthMeters=1000*2**l,tileCount=len(keys),baseVertices=sum(r['baseVertices'] for r in rows),addedVertices=sum(r['addedVertices'] for r in rows),vertices=sum(r['vertices'] for r in rows),triangles=sum(r['triangles'] for r in rows),bytes=sum(r['bytes'] for r in rows),baseUnmet=sum(not r['baseToleranceReached'] for r in rows),unmet=sum(not r['toleranceReached'] for r in rows),maxErrorMeters=max(r['maxErrorMeters'] for r in rows),maxErrorRatio=max(r['maxErrorRatio'] for r in rows),sampleSpacingMeters=rows[0]['sampleSpacingMeters'],addedPositions=[p for r in rows for p in r['addedPositions']],keys=keys))
   cx,cy=national.project(lon,lat);height,_=national.source_samples(np.array([cx]),np.array([cy]))
   regions.append(dict(id=id,name=name,lon=lon,lat=lat,height=float(height[0]),levels=levels))
  manifest=dict(curveAware=os.environ.get('TERRAIN_CURVE_AWARE')=='1',curveRefinementPolicy='edge-hermite-dem-v1',boundaryPolicy='adaptive-half-error-v1',regionalVersion=3 if os.environ.get('TERRAIN_CURVE_AWARE')=='1' else 2,regionalLevels=True,levels=specifications(),reliefRegions=regions,priorityMaximumWeight=2)
  db.execute('INSERT OR REPLACE INTO metadata VALUES(?,?)',('manifest',json.dumps(manifest)));db.commit()
  (ROOT/'build/terrain-regional-priority-report.json').write_text(json.dumps(manifest,ensure_ascii=False,indent=2),encoding='utf-8')
 print('全レベル完了',OUTPUT,flush=True)

if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--workers',type=int,default=2);p.add_argument('--binary',type=Path);p.add_argument('--curve-aware',action='store_true');p.add_argument('--output',type=Path);a=p.parse_args()
 if a.curve_aware:os.environ['TERRAIN_CURVE_AWARE']='1'
 if a.output:OUTPUT=a.output.resolve();os.environ['TERRAIN_REGIONAL_OUTPUT']=str(OUTPUT)
 if a.curve_aware and not a.output and OUTPUT.name=='terrain-regional-priority-levels-v2.sqlite3':OUTPUT=ROOT/'build/terrain-regional-priority-levels-curve.sqlite3';os.environ['TERRAIN_REGIONAL_OUTPUT']=str(OUTPUT)
 generate(a.workers,a.binary)
