"""共有境界を同期し、Delaunay面の小さすぎる角を追加点で改善する。"""
import argparse,json,math,sqlite3,struct,zlib
from pathlib import Path
import numpy as np
from scipy.spatial import Delaunay
from scipy.ndimage import map_coordinates
import national_independent_mesh as national
from relative_mesh_preview import error_limits,ridge_limits
from national_mesh_levels import ERRORS
from curve_mesh_refinement import mesh_errors
ROOT=national.ROOT

def triangulate(points):
 f=Delaunay(np.asarray(points)[:,:2]).simplices.copy()
 p=np.asarray(points);a=p[f[:,1],:2]-p[f[:,0],:2];b=p[f[:,2],:2]-p[f[:,0],:2];negative=a[:,0]*b[:,1]-a[:,1]*b[:,0]<0
 f[negative]=f[negative][:,[0,2,1]]
 return f

def angles(points,faces):
 t=np.asarray(points)[faces,:2];result=[]
 for i in range(3):
  a=t[:,(i+1)%3]-t[:,i];b=t[:,(i+2)%3]-t[:,i];result.append(np.degrees(np.arccos(np.clip((a*b).sum(axis=1)/(np.linalg.norm(a,axis=1)*np.linalg.norm(b,axis=1)),-1,1))))
 return np.min(result,axis=0)

def boundary_segments(points,n):
 p=np.asarray(points);segments=[]
 for axis,value in [(0,0),(0,n-1),(1,0),(1,n-1)]:
  ids=np.flatnonzero(p[:,axis]==value);ids=ids[np.argsort(p[ids,1-axis])]
  segments.extend((p[a,:2],p[b,:2]) for a,b in zip(ids[:-1],ids[1:]))
 return segments

def quality_candidates(points,faces,n,minimum=10,maximum=64):
 """鋭角面の外接円中心を選び、境界を侵す場合は境界の中点を先に選ぶ。"""
 p=np.asarray(points);existing={tuple(map(int,v[:2])) for v in p};segments=boundary_segments(p,n);a=np.array([s[0] for s in segments]);b=np.array([s[1] for s in segments]);out={};mins=angles(p,faces)
 for face in np.argsort(mins):
  if mins[face]>=minimum:break
  t=p[faces[face],:2];u=t[1]-t[0];v=t[2]-t[0];det=2*(u[0]*v[1]-u[1]*v[0]);uu=u@u;vv=v@v
  q=t[0]+np.array([(uu*v[1]-vv*u[1])/det,(u[0]*vv-v[0]*uu)/det])
  encroached=((q-a)*(q-b)).sum(axis=1)<-1e-8
  outside=np.any(q<=0) or np.any(q>=n-1)
  if encroached.any() or outside:
   candidates=np.flatnonzero(encroached)
   if not len(candidates):
    ab=b-a;at=np.clip(((q-a)*ab).sum(axis=1)/(ab*ab).sum(axis=1),0,1);candidates=[int(np.argmin(np.linalg.norm(q-(a+at[:,None]*ab),axis=1)))]
   k=max(candidates,key=lambda k:np.linalg.norm(b[k]-a[k]));q=(a[k]+b[k])/2
  xy=tuple(map(int,np.rint(q)))
  if xy in existing or min(xy)<0 or max(xy)>=n:continue
  out[xy]=min(out.get(xy,180),float(mins[face]))
  if len(out)>=maximum:break
 return sorted(out,key=lambda xy:(out[xy],xy))

def append_point(tile,xy):
 if xy in tile['existing']:return False
 if tile['added']>=tile['budget']:return False
 x,y=xy;tile['points'].append([x,y,float(tile['grid'][y,x])]);tile['existing'].add(xy);tile['added']+=1;return True

def refine_group(tiles,minimum=10,rounds=32):
 """同じレベルの全タイルを一緒に処理し、境界の追加点を両側へ配る。"""
 for tile in tiles.values():
  tile['existing']={tuple(map(int,p[:2])) for p in tile['points']};tile.setdefault('added',0)
 for iteration in range(rounds):
  requests={};inserted=0
  for key,t in tiles.items():
   t['faces']=triangulate(t['points'])
   if t['added']>=t['budget']:continue
   for xy in quality_candidates(t['points'],t['faces'],t['n'],minimum):
    if xy[0] in [0,t['n']-1] or xy[1] in [0,t['n']-1]:
     gx=key[0]*(t['n']-1)+xy[0];gy=key[1]*(t['n']-1)+xy[1];requests[gx,gy]=True
    elif append_point(t,xy):inserted+=1
  for gx,gy in sorted(requests):
   targets=[]
   for (x,y),t in tiles.items():
    xy=(gx-x*(t['n']-1),gy-y*(t['n']-1))
    if 0<=xy[0]<t['n'] and 0<=xy[1]<t['n']:targets.append((t,xy))
   if any(xy not in t['existing'] and t['added']>=t['budget'] for t,xy in targets):continue
   for t,xy in targets:inserted+=append_point(t,xy)
  if not inserted:break
 for t in tiles.values():t['faces']=triangulate(t['points'])
 return iteration+1

def source_fields(level,x,y,n,*,width_meters=None,error_meters=None):
 width=1000*2**level if width_meters is None else width_meters;error=ERRORS[level] if error_meters is None else error_meters;spacing=width/(n-1);axis=np.arange(n)*spacing;xx,yy=np.meshgrid(x*width+axis,y*width+axis);grid,_=national.source_samples(xx,yy);grid=np.rint(grid*4)/4
 field_spacing=max(62.5,spacing);pad=math.ceil(6000/field_spacing);core=round(width/field_spacing)+1;axis=(np.arange(core+2*pad)-pad)*field_spacing;xx,yy=np.meshgrid(x*width+axis,(y+1)*width-axis);halo,valid=national.source_samples(xx,yy)
 reference=error_limits(halo,valid,field_spacing,minimum=max(.25,error*.1),maximum=error);weight=np.clip(1-ridge_limits(halo,valid,field_spacing,np.zeros_like(halo))/50,0,1);field=error*(1-weight)+weight*reference/(1+weight)
 fy,fx=np.meshgrid((n-1-np.arange(n))*spacing/field_spacing+pad,np.arange(n)*spacing/field_spacing+pad,indexing='ij');limits=np.maximum(.01,np.rint(map_coordinates(field,[fy,fx],order=1)*100)/100)
 gx=map_coordinates(np.gradient(halo,field_spacing,axis=1),[fy,fx],order=1);gy=-map_coordinates(np.gradient(halo,field_spacing,axis=0),[fy,fx],order=1)
 return grid,limits,gx,gy

def error_candidates(points,faces,grid,tolerance,maximum=64):
 candidates={};n=len(grid);p=np.asarray(points)
 for face in faces:
  a,b,c=p[face];ab=b[:2]-a[:2];ac=c[:2]-a[:2];det=ab[0]*ac[1]-ab[1]*ac[0];lo=np.maximum(0,np.ceil(np.minimum(np.minimum(a[:2],b[:2]),c[:2]))).astype(int);hi=np.minimum(n-1,np.floor(np.maximum(np.maximum(a[:2],b[:2]),c[:2]))).astype(int)
  yy,xx=np.mgrid[lo[1]:hi[1]+1,lo[0]:hi[0]+1];u=((xx-a[0])*ac[1]-(yy-a[1])*ac[0])/det;v=(ab[0]*(yy-a[1])-ab[1]*(xx-a[0]))/det
  inside=(u>=-1e-8)&(v>=-1e-8)&(u+v<=1+1e-8)&(xx>0)&(yy>0)&(xx<n-1)&(yy<n-1);err=np.where(inside,np.abs(a[2]+u*(b[2]-a[2])+v*(c[2]-a[2])-grid[yy,xx]),-1)
  if err.size and err.max()>tolerance+1e-6:
   at=np.unravel_index(np.argmax(err),err.shape);xy=(int(xx[at]),int(yy[at]));candidates[xy]=float(err[at])
 return sorted(candidates,key=lambda xy:(-candidates[xy],xy))[:maximum]

def generate(source,output,levels):
 if not levels or any(l not in range(8) for l in levels):raise ValueError('対象レベルは0～7を指定してください')
 if source.resolve()==output.resolve():raise ValueError('元データとは別の出力ファイルを指定してください')
 if output.exists():raise ValueError('出力ファイルは既に存在します')
 national.initialize(ROOT/'build/terrain-lod.sqlite3',str(ROOT/'build/terrain-rust-target/debug/terrain-adaptive-tin.exe'))
 with sqlite3.connect(source) as src,sqlite3.connect(output) as db:
  src.backup(db);manifest=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
  for level in levels:
   tiles={}
   for x,y,g,h,m,report in db.execute('SELECT x,y,geometry,hints,mask,stats FROM tiles WHERE level=?',(level,)):
    raw=zlib.decompress(g);_,n,_,nv,ni=struct.unpack_from('<4sHHII',raw);p=np.frombuffer(raw,dtype='<f4',offset=16,count=nv*3).reshape(-1,3).astype(float);p[:,:2]/=1000*2**level/(n-1);oldf=np.frombuffer(raw,dtype='<u2',offset=16+nv*12).reshape(-1,3);r=json.loads(report);grid,limits,gx,gy=source_fields(level,x,y,n)
    tiles[x,y]=dict(points=p.tolist(),oldPoints=p.copy(),oldHints=zlib.decompress(h),oldFaces=oldf,stats=r,mask=m,n=n,grid=grid,limits=limits,gx=gx,gy=gy,added=0,budget=min(1024,16384-r['baseVertices']),beforeUnder10=int((angles(p,oldf)<10).sum()))
   refine_group(tiles)
   # 品質の改善で標高誤差が増えた場所を、残りの基本枠で補う。
   for t in tiles.values():
    t['budget']=min(2048,16384-t['stats']['baseVertices'])
    for iteration in range(16):
     candidates=error_candidates(t['points'],t['faces'],t['grid'],max(ERRORS[level],t['stats']['maxErrorMeters']))
     if not candidates:break
     count=sum(append_point(t,xy) for xy in candidates)
     if not count:break
     t['faces']=triangulate(t['points'])
   for (x,y),t in tiles.items():
    p=np.array(t['points']);f=t['faces'];r=t['stats'];n=t['n'];width=1000*2**level;spacing=width/(n-1);maximum,ratio=mesh_errors(p,f,t['grid'],t['limits']);allowed=max(ERRORS[level],r['maxErrorMeters'])
    if maximum>allowed+1e-5:raise ValueError(f'標高誤差が増えました L{level}/{x}/{y}: {allowed} -> {maximum}')
    world=p.copy();world[:,:2]*=spacing;g=zlib.compress(struct.pack('<4sHHII',b'YNT2',n,n,len(p),f.size)+world.astype('<f4').tobytes()+f.astype('<u2').tobytes(),9)
    hints=np.column_stack([map_coordinates(t['gx'],[p[:,1],p[:,0]],order=1),-map_coordinates(t['gy'],[p[:,1],p[:,0]],order=1)]);encoded=np.rint(np.clip(hints,-8,8)/8*32767).astype('<i2');old=np.frombuffer(t['oldHints'],dtype='<i2',offset=12).reshape(-1,2);encoded[:len(old)]=old
    h=zlib.compress(struct.pack('<4sIf',b'YNH1',len(p),8.)+encoded.tobytes(),9)
    edges={}
    for name,axis,value,coordinate in [('north',1,n-1,0),('south',1,0,0),('west',0,0,1),('east',0,n-1,1)]:
     selected=p[p[:,axis]==value];at=selected[:,coordinate] if coordinate==0 else n-1-selected[:,coordinate];order=np.argsort(at);edges[name]=[[int(at[i]),float(selected[i,2])] for i in order]
    minimums=angles(p,f);r.update(qualityRefined=True,qualityMinimumAngleTarget=10,qualityAddedVertices=t['added'],beforeUnder10=t['beforeUnder10'],afterUnder10=int((minimums<10).sum()),minimumAngleDegrees=float(minimums.min()),vertices=len(p),triangles=len(f),baseVertices=r['baseVertices']+t['added'],maxErrorMeters=maximum,maxErrorRatio=ratio,toleranceReached=ratio<=1+1e-8,bytes=len(g)+len(h)+len(t['mask']),edges=edges,boundaryVertices=sum(1 for v in p if v[0] in [0,n-1] or v[1] in [0,n-1]))
    assert r['baseVertices']<=16384 and r['vertices']==r['baseVertices']+r['addedVertices']
    db.execute('UPDATE tiles SET geometry=?,hints=?,stats=? WHERE level=? AND x=? AND y=?',(g,h,json.dumps(r),level,x,y))
   db.commit();print('L',level,'完了','鋭角面',sum(t['beforeUnder10'] for t in tiles.values()),'->',sum(t['stats']['afterUnder10'] for t in tiles.values()),'追加',sum(t['added'] for t in tiles.values()),flush=True)
  finalize_quality(db,manifest,levels)
 print('完了',output,flush=True)

def finalize_quality(db,manifest,levels):
 # 追加された共有点の後でも、境界の実際の標高誤差を測り直す。
 for level,x,y,report in db.execute('SELECT level,x,y,stats FROM tiles').fetchall():
  r=json.loads(report)
  if not r.get('qualityRefined'):continue
  width=1000*2**level;n=round(width/r['sampleSpacingMeters'])+1;axis=np.arange(n)*r['sampleSpacingMeters'];maximum=0.
  for name,xx,yy in [('north',x*width+axis,np.full(n,(y+1)*width)),('south',x*width+axis,np.full(n,y*width)),('west',np.full(n,x*width),(y+1)*width-axis),('east',np.full(n,(x+1)*width),(y+1)*width-axis)]:
   truth,_=national.source_samples(xx,yy);truth=np.rint(truth*4)/4;edge=np.array(r['edges'][name]);maximum=max(maximum,float(np.abs(np.interp(np.arange(n),edge[:,0],edge[:,1])-truth).max()))
  assert maximum<=max(ERRORS[level],r['maxErrorMeters'])+1e-5
  r['boundaryPolicy']='quality-synchronized-v1';r['boundaryActualMaxErrorMeters']=maximum;r['boundaryErrorMeters']=max(r['boundaryErrorMeters'],maximum)
  db.execute('UPDATE tiles SET stats=? WHERE level=? AND x=? AND y=?',(json.dumps(r),level,x,y))
 for area in manifest['reliefRegions']:
  for l in area['levels']:
   rows=[json.loads(db.execute('SELECT stats FROM tiles WHERE level=? AND x=? AND y=?',key).fetchone()[0]) for key in l['keys']]
   for field in ['baseVertices','addedVertices','vertices','triangles','bytes']:l[field]=sum(r[field] for r in rows)
   l['maxErrorMeters']=max(r['maxErrorMeters'] for r in rows);l['maxErrorRatio']=max(r['maxErrorRatio'] for r in rows);l['unmet']=sum(not r['toleranceReached'] for r in rows)
   l['qualityBeforeUnder10']=sum(r.get('beforeUnder10',0) for r in rows);l['qualityAfterUnder10']=sum(r.get('afterUnder10',0) for r in rows)
 manifest.update(qualityRefinedLevels=levels,qualityMinimumAngleTarget=10,regionalVersion=4);db.execute('UPDATE metadata SET value=? WHERE key="manifest"',(json.dumps(manifest),));db.commit()

if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--source',type=Path,default=ROOT/'build/terrain-regional-priority-levels-v4.sqlite3');p.add_argument('--output',type=Path,required=True);p.add_argument('--levels',type=int,nargs='+',default=list(range(8)));a=p.parse_args();generate(a.source,a.output,a.levels)
