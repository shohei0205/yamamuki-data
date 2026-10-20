"""固定誤差と周辺比高による誤差を、同じ入力・頂点上限で比較生成する。"""
import json,sqlite3,struct,subprocess,tempfile,zlib
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.ndimage import maximum_filter,minimum_filter,distance_transform_edt,gaussian_filter
import national_independent_mesh as national
from tin_vertex_optimize import raster
from peak_error import summit_limits
from mesh_grading import graded_sizes

ROOT=national.ROOT
REGIONS=[('fuji','富士山',138.7274,35.3606),('tsukuba','筑波山',140.1069,36.2254),('yari','槍ヶ岳',137.6476,36.3419)]

def error_limits(halo,valid,spacing,radii=(1000,4000),ratio=.05,minimum=2.,maximum=50.,flat_guard=False,smoothing_meters=500.):
 """物理距離の窓で求める。欠損をゼロ標高の谷として扱わない。"""
 if not valid.any():raise ValueError('有効な標高がありません')
 filled=halo if valid.all() else halo[tuple(distance_transform_edt(~valid,return_distances=False,return_indices=True))]
 small=2*int(round(radii[0]/spacing))+1;large=2*int(round(radii[1]/spacing))+1
 a=maximum_filter(filled,size=small,mode='nearest')-minimum_filter(filled,size=small,mode='nearest')
 b=maximum_filter(filled,size=large,mode='nearest')-minimum_filter(filled,size=large,mode='nearest')
 relief=np.maximum(a,.5*b)
 if smoothing_meters>0:relief=gaussian_filter(relief,sigma=smoothing_meters/spacing,mode="nearest",truncate=4.)
 allowed=np.clip(relief*ratio,minimum,maximum)
 if flat_guard:
  t=np.clip(relief/100,0,1);allowed=np.maximum(allowed,10*(1-t*t*(3-2*t)))
 return allowed

def ridge_limits(heights,valid,spacing,relative):
 """広い尾根と山頂の上部へ精度を配り、山腹は50mに戻す。"""
 filled=heights if valid.all() else heights[tuple(distance_transform_edt(~valid,return_distances=False,return_indices=True))]
 surface=gaussian_filter(filled,250/spacing,mode='nearest')
 window=2*int(round(1500/spacing))+1
 low=minimum_filter(surface,size=window,mode='nearest');high=maximum_filter(surface,size=window,mode='nearest')
 position=(surface-low)/np.maximum(high-low,1.)
 # 直交する断面のうち、最も凸な方向で尾根を拾う。細かな溝は250m平滑面で除く。
 step=max(1,int(round(500/spacing)));padded=np.pad(surface,step,mode='edge');h,w=surface.shape;crest=np.zeros_like(surface)
 for dy,dx in [(1,0),(0,1),(1,1),(1,-1)]:
  a=padded[step+dy*step:step+dy*step+h,step+dx*step:step+dx*step+w]
  b=padded[step-dy*step:step-dy*step+h,step-dx*step:step-dx*step+w]
  crest=np.maximum(crest,surface-(a+b)*.5)
 t=np.clip((position-.55)/.3,0,1);upper=t*t*(3-2*t)
 t=np.clip(crest/np.maximum((high-low)*.05,2.),0,1);convex=t*t*(3-2*t)
 weight=gaussian_filter(upper*convex,350/spacing,mode='nearest')
 return np.rint((50-weight*(50-relative))*100)/100

def priority_limits(heights,valid,spacing,maximum_weight=2.):
 """誤差/比高基準×地形重みを、Rust生成器の誤差倍率へ変換する。"""
 reference=error_limits(heights,valid,spacing,minimum=5.)
 weight=np.clip(1-ridge_limits(heights,valid,spacing,np.zeros_like(heights))/50,0,1)
 return np.rint((50*(1-weight)+weight*reference/(1+(maximum_weight-1)*weight))*100)/100

def settings_for(kind):
 return dict(mode=kind,reliefPriority=kind=='priority1024',priorityMinimumMeters=5,priorityMaximumMeters=50,priorityMaximumWeight=2,additionalVertexLimit=1024 if kind in ['ridge1024','priority1024'] else None,ridgePriority=kind in ['ridge','ridge1024','priority1024'],ratio=.05,minimumMeters=2,maximumMeters=50,radiiMeters=[1000,4000],largeRangeWeight=.5,reliefSmoothingMeters=500,sampleSpacingMeters=15.625,flatGuard=kind=='guard',flatGuardMaximumMeters=10,peakPriority=kind in ['peaks','graded','neighbor0','neighbor3','neighbor2'],meshGrading=kind=='graded',meshGrowth=.3,meshMinimumRadiusMeters=64,peakErrorMeters=25,flankErrorMeters=25,minimumProminenceMeters=20,flatErrorMeters=10,neighborBalancing=kind.startswith('neighbor'),neighborRatio=2 if kind=='neighbor2' else 3,additionalFraction=0 if kind=='neighbor0' else .15)

def generate(kinds=None,regions=None):
 binary=ROOT/'build/terrain-rust-target/release/terrain-adaptive-tin.exe';national.initialize(ROOT/'build/terrain-lod.sqlite3',str(binary))
 outputs={kind:ROOT/f'build/terrain-relief-{kind}-l3.sqlite3' for kind in (kinds or ['fixed','relative','guard','peaks','graded','neighbor0','neighbor3','neighbor2','ridge','ridge1024','priority1024'])}
 with sqlite3.connect(ROOT/'build/terrain-independent-national.sqlite3') as old:
  base=json.loads(old.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0]);reports={k:[] for k in outputs}
  for kind,path in outputs.items():
   with sqlite3.connect(path) as db:
    db.executescript('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE IF NOT EXISTS tiles(level INTEGER,x INTEGER,y INTEGER,geometry BLOB,hints BLOB,mask BLOB,stats TEXT,PRIMARY KEY(level,x,y));')
    if regions:
     saved=db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()
     if saved:
      previous=json.loads(saved[0]);reports[kind]=previous.get('reliefRegions',[])
      for record in reports[kind]:record.setdefault('settings',previous.get('reliefSettings',{}))
    else:db.execute('DELETE FROM tiles')
  for region,name,lon,lat in REGIONS:
   if regions and region not in regions:continue
   width=8000;n=2049;spacing=32000/(n-1);pad=int(6000/spacing);cx,cy=national.project(lon,lat);tx=int(cx//width)-1;ty=int(cy//width)-1
   axis=np.arange(-pad,n+pad)*spacing;xx,yy=np.meshgrid(tx*width+axis,(ty+4)*width-axis)
   print(name,'周辺6 kmを含む元DEM取得',flush=True);halo,valid=national.source_samples(xx,yy);del xx,yy
   limit=error_limits(halo,valid,spacing)[pad:pad+n,pad:pad+n];limit=np.rint(limit*100)/100
   guarded=error_limits(halo,valid,spacing,flat_guard=True)[pad:pad+n,pad:pad+n];guarded=np.rint(guarded*100)/100
   print(name,'ピークと鞍部を抽出',flush=True)
   summit,peaks=summit_limits(halo,valid,spacing);summit=summit[pad:pad+n,pad:pad+n];seeds=[[p['x']-pad,p['y']-pad] for p in peaks if pad<=p['x']<pad+n and pad<=p['y']<pad+n];peak_records=[p for p in peaks if pad<=p['x']<pad+n and pad<=p['y']<pad+n]
   ridge=ridge_limits(halo,valid,spacing,error_limits(halo,valid,spacing))[pad:pad+n,pad:pad+n]
   priority=priority_limits(halo,valid,spacing)[pad:pad+n,pad:pad+n]
   grid=np.rint(halo[pad:pad+n,pad:pad+n]*4)/4;mask=valid[pad:pad+n,pad:pad+n];dy,dx=np.gradient(halo,spacing)
   with tempfile.TemporaryDirectory(dir=ROOT/'build',prefix='relief-tin-') as temp:
    temp=Path(temp);offset=min(0,float(grid.min()));Image.fromarray(np.rint((grid-offset)*4).astype(np.uint16)).save(temp/'height.png');Image.fromarray(np.rint(limit*100).astype(np.uint16)).save(temp/'limits.png')
    Image.fromarray(np.rint(summit*100).astype(np.uint16)).save(temp/'peaks.png');(temp/'seeds.json').write_text(json.dumps(seeds),encoding='utf-8')
    for kind,path in outputs.items():
     initial_cap=n*n if kind in ['ridge1024','priority1024'] else 65536 if kind.startswith('neighbor') else 32768
     report=temp/f'{kind}.json';cmd=[str(binary),'--input',str(temp/'height.png'),'--output',str(report),'--scale','.25','--offset',str(offset),'--min-vertices','4' if kind in ['ridge1024','priority1024'] else '128','--max-vertices',str(initial_cap),'--max-error','50']
     if kind not in ['fixed','ridge1024','priority1024']:
      if kind in ['ridge','ridge1024','priority1024']:Image.fromarray(np.rint(ridge*100).astype(np.uint16)).save(temp/'ridge.png')
      if kind=='guard':Image.fromarray(np.rint(guarded*100).astype(np.uint16)).save(temp/'guarded.png')
      cmd+=['--error-limits',str(temp/('peaks.png' if kind in ['peaks','graded','neighbor0','neighbor3','neighbor2'] else 'ridge.png' if kind in ['ridge','ridge1024','priority1024'] else 'guarded.png' if kind=='guard' else 'limits.png'))]
      if kind in ['peaks','graded','neighbor0','neighbor3','neighbor2']:cmd+=['--seed-points',str(temp/'seeds.json')]
     subprocess.run(cmd,check=True);mesh=json.loads(report.read_text(encoding='utf-8'));p=np.asarray(mesh['vertices']);f=np.asarray(mesh['triangles'],np.uint32)
     budget=initial_cap
     base_vertices=None;base_error=None
     if kind in ['ridge1024','priority1024']:
      base_vertices=p.copy();base_error=mesh['maxError'];assert base_error<=50+1e-8
      budget=len(p)+1024
      Image.fromarray(np.rint((priority if kind=='priority1024' else ridge)*100).astype(np.uint16)).save(temp/'ridge.png')
      (temp/'base-seeds.json').write_text(json.dumps(p[:,:2].astype(int).tolist()),encoding='utf-8')
      refined=cmd.copy();refined[refined.index('--max-vertices')+1]=str(budget)
      refined+=['--error-limits',str(temp/'ridge.png'),'--seed-points',str(temp/'base-seeds.json')]
      subprocess.run(refined,check=True);mesh=json.loads(report.read_text(encoding='utf-8'));p=np.asarray(mesh['vertices']);f=np.asarray(mesh['triangles'],np.uint32)
      assert len(p)<=budget and {(int(v[0]),int(v[1]),float(v[2])) for v in base_vertices}.issubset({(int(v[0]),int(v[1]),float(v[2])) for v in p})
     if kind=='graded':
      print(name,'密度を段階的に調整',flush=True);sizes=graded_sizes(p,f,n,spacing)
      Image.fromarray(np.rint(sizes*100).astype(np.uint16)).save(temp/'sizes.png');(temp/'graded-seeds.json').write_text(json.dumps(p[:,:2].astype(int).tolist()),encoding='utf-8')
      budget=65536;graded_cmd=cmd.copy();graded_cmd[graded_cmd.index('--max-vertices')+1]=str(budget);graded_cmd[graded_cmd.index('--seed-points')+1]=str(temp/'graded-seeds.json');graded_cmd+=['--mesh-sizes',str(temp/'sizes.png')]
      subprocess.run(graded_cmd,check=True);mesh=json.loads(report.read_text(encoding='utf-8'));p=np.asarray(mesh['vertices']);f=np.asarray(mesh['triangles'],np.uint32)
     if kind.startswith('neighbor'):
      initial_vertices=len(p);budget=initial_vertices+(0 if kind=='neighbor0' else initial_vertices*15//100)
      (temp/'neighbor-seeds.json').write_text(json.dumps(p[:,:2].astype(int).tolist()),encoding='utf-8')
      neighbor_cmd=cmd.copy();neighbor_cmd[neighbor_cmd.index('--max-vertices')+1]=str(budget);neighbor_cmd[neighbor_cmd.index('--seed-points')+1]=str(temp/'neighbor-seeds.json');neighbor_cmd+=['--neighbor-ratio','2' if kind=='neighbor2' else '3']
      subprocess.run(neighbor_cmd,check=True);mesh=json.loads(report.read_text(encoding='utf-8'));p=np.asarray(mesh['vertices']);f=np.asarray(mesh['triangles'],np.uint32)
      assert mesh['balance']['initialVertices']==initial_vertices
      assert len(p)<=budget
     owner,weights=raster(p,f,n);pred=(p[f[owner],2]*weights).sum(axis=1);errors=np.abs(pred-grid.ravel());allowed=(summit if kind in ['peaks','graded','neighbor0','neighbor3','neighbor2'] else priority if kind=='priority1024' else ridge if kind in ['ridge','ridge1024'] else guarded if kind=='guard' else limit).ravel() if kind!='fixed' else np.full(grid.size,50.)
     maximum=float(errors[mask.ravel()].max());max_ratio=float((errors/allowed)[mask.ravel()].max());size_ratio=mesh.get('maxMeshSizeRatio');satisfied=max_ratio<=1+1e-8 and (size_ratio is None or size_ratio<=1+1e-8)
     # 低い起伏と平坦な場所の誤差を、両方式で同じ領域に対して集計する。
     low=mask.ravel()&(limit.ravel()>2)&(limit.ravel()<=15);flat=mask.ravel()&(limit.ravel()==2)
     def measure(which):return dict(samples=int(which.sum()),maxError=float(errors[which].max()) if which.any() else None,rmse=float(np.sqrt(np.mean(errors[which]**2))) if which.any() else None)
     world=p.copy();world[:,0]=tx*width+p[:,0]*spacing;world[:,1]=(ty+4)*width-p[:,1]*spacing;f=f[:,[0,2,1]];centers=world[f,:2].mean(axis=1);owned=np.floor(centers/width).astype(int)
     gx=p[:,0].astype(int)+pad;gy=p[:,1].astype(int)+pad;slopes=np.column_stack([dx[gy,gx],dy[gy,gx]])
     total=0;vcounts=[]
     with sqlite3.connect(path) as db:
      for x in range(tx,tx+4):
       for y in range(ty,ty+4):
        chosen=f[(owned[:,0]==x)&(owned[:,1]==y)];ids,indices=np.unique(chosen,return_inverse=True);q=world[ids].copy();q[:,:2]-=[x*width,y*width];assert len(ids)<65536
        geometry=zlib.compress(struct.pack('<4sHHII',b'YNT2',1025,1025,len(ids),indices.size)+q.astype('<f4').tobytes()+indices.astype('<u2').tobytes(),9)
        h=zlib.compress(struct.pack('<4sIf',b'YNH1',len(ids),8.)+np.rint(np.clip(slopes[ids],-8,8)/8*32767).astype('<i2').tobytes(),9)
        saved_mask=old.execute('SELECT mask FROM tiles WHERE level=3 AND x=? AND y=?',(x,y)).fetchone()[0]
        db.execute('INSERT OR REPLACE INTO tiles VALUES(?,?,?,?,?,?,?)',(3,x,y,geometry,h,saved_mask,json.dumps(dict(vertices=len(ids),triangles=len(chosen),joint=True))))
        total+=len(geometry)+len(h);vcounts.append(len(ids))
      db.commit()
     if kind in ['peaks','graded','neighbor0','neighbor3','neighbor2']:
      selected={(int(v[0]),int(v[1])) for v in p};assert all(tuple(seed) in selected for seed in seeds),'山頂が保存されていません'
     crown_grid=np.zeros((n,n),dtype=bool)
     for peak in peak_records:
      px=peak['x']-pad;py=peak['y']-pad;r=peak['radiusMeters']*.4;cells=int(np.ceil(r/spacing));xa=max(0,px-cells);xb=min(n,px+cells+1);ya=max(0,py-cells);yb=min(n,py+cells+1);yy,xx=np.mgrid[ya:yb,xa:xb]
      crown_grid[ya:yb,xa:xb]|=(np.hypot(xx-px,yy-py)*spacing<=r)&(grid[ya:yb,xa:xb]>=peak['height']-peak['crownDropMeters']*.4)
     crown=mask.ravel()&crown_grid.ravel()
     peak_positions=[(v['y']-pad)*n+v['x']-pad for v in peak_records]
     peak_max=float(errors[peak_positions].max()) if peak_positions else 0.
     base_ids={(int(v[0]),int(v[1])) for v in base_vertices} if base_vertices is not None else set()
     added_positions=[world[i].tolist() for i,v in enumerate(p) if (int(v[0]),int(v[1])) not in base_ids] if base_vertices is not None else []
     assert base_vertices is None or len(added_positions)==len(p)-len(base_vertices)
     data=dict(addedPositions=added_positions,baseVertices=len(base_vertices) if base_vertices is not None else None,baseMaxErrorMeters=base_error,addedVertices=len(p)-len(base_vertices) if base_vertices is not None else None,balance=mesh.get('balance'),initialCap=initial_cap,settings=settings_for(kind),maxMeshSizeRatio=size_ratio,peakCount=len(seeds),peakHeightMaxError=peak_max,crown=measure(crown),id=region,name=name,lon=lon,lat=lat,bounds=[tx*width,(tx+4)*width,ty*width,(ty+4)*width],vertices=len(p),triangles=len(f),maxVertices=budget,minVertices=4 if kind in ['ridge1024','priority1024'] else 128,bytes=total,maxErrorMeters=maximum,maxErrorRatio=max_ratio,toleranceReached=satisfied,lowRelief=measure(low),flat=measure(flat),minVerticesPerTile=min(vcounts),maxVerticesPerTile=max(vcounts))
     reports[kind]=[record for record in reports[kind] if record['id']!=region];reports[kind].append(data);print(kind,name,json.dumps({k:v for k,v in data.items() if k!='addedPositions'},ensure_ascii=False),flush=True)
  for kind,path in outputs.items():
   meta=dict(base);meta['reliefRegions']=reports[kind];meta['reliefSettings']=settings_for(kind)
   with sqlite3.connect(path) as db:db.execute('INSERT OR REPLACE INTO metadata VALUES(?,?)',('manifest',json.dumps(meta)))
  summary=ROOT/'build/terrain-relief-report.json';previous=json.loads(summary.read_text(encoding='utf-8')) if summary.exists() else {};previous.update(reports);summary.write_text(json.dumps(previous,ensure_ascii=False,indent=2),encoding='utf-8')
 from mesh_added_vertices import update
 update(list(outputs.values()))
 return reports
if __name__=='__main__':
 import argparse
 parser=argparse.ArgumentParser();parser.add_argument('--modes',nargs='+',choices=['fixed','relative','guard','peaks','graded','neighbor0','neighbor3','neighbor2','ridge','ridge1024','priority1024']);parser.add_argument('--regions',nargs='+',choices=[r[0] for r in REGIONS]);args=parser.parse_args();generate(args.modes,args.regions)
