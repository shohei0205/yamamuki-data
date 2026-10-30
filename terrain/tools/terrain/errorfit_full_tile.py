"""元DEMをPNG区画単位で走査する、全域の誤差駆動Sibson適合。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
import json,time,math
from pathlib import Path
import numpy as np
from scipy.stats import qmc
from natural_c1_128 import ROOT,RUN as BASE,write,GlobalC1,close_cells,pack
from continuous_dem import tile,STEP
from sibson_aux_surface import AuxiliarySibsonSurface
from errorfit_seed import corner_support
from streamed_dem_prune import DemReference
OUT=ROOT/'build/natural-raw-c1-128-errorfit-full-v1'

def save_points(path,p,support_count=0):
 temporary=path.with_name(path.name+'.tmp.npz');np.savez_compressed(temporary,points=p,support_count=np.asarray(support_count));temporary.replace(path)

def influence(model,added):
 circles=model.circles;boxes=[]
 for p in added:
  d=circles[:,:2]-p[:2];mask=np.sum(d*d,axis=1)<=circles[:,2]*(1+1e-10);c=circles[mask]
  if len(c):
   radius=np.sqrt(c[:,2]);boxes.append([np.min(c[:,0]-radius),np.min(c[:,1]-radius),np.max(c[:,0]+radius),np.max(c[:,1]+radius)])
  else:boxes.append([p[0]-64,p[1]-64,p[0]+64,p[1]+64])
 return boxes

def build(region,error,x,y,source_error=None,seed_policy="legacy",max_iterations=512,pack_depth=11,controls_only=False):
 if seed_policy not in ["legacy","dem-corners"]:raise ValueError("不正な初期点方式")
 batch_size=1 if seed_policy=="dem-corners" else 64
 started=time.monotonic();src=BASE/region/f'{(error if source_error is None else source_error):g}'/f'{x}-{y}';dst=OUT/region/f'{error:g}'/f'{x}-{y}';dst.mkdir(parents=True,exist_ok=True)
 if (dst/'selection.json').exists():return json.loads((dst/'selection.json').read_text())
 info=json.loads((src/'packet.json').read_text());info['error']=error;side=info['side'];halo=info.get('halo',max(500,side/8));world=np.asarray(json.loads((BASE/region/'source/region.json').read_text())['origin'])+np.asarray(info['origin']);half=side/2+250;lo=np.ceil((world-half)/STEP-.5).astype(int);hi=np.floor((world+half)/STEP-.5).astype(int)
 keys=[(tx,ty) for ty in range(int(lo[1]//256),int(hi[1]//256)+1) for tx in range(int(lo[0]//256),int(hi[0]//256)+1)];cache={};dirty=set(keys);history=[];final_audit=False
 if info.get('empty'):
  # 原本が空でも、今回の監査範囲を全走査して有効画素が無いことを確認。
  valid=sum(np.isfinite(tile(tx,ty)[max(0,lo[1]-ty*256):min(256,hi[1]-ty*256+1),max(0,lo[0]-tx*256):min(256,hi[0]-tx*256+1)]).sum() for tx,ty in keys)
  if valid:raise ValueError('原本は空だが接合帯内に有効画素がある')
  out=dict(info,empty=True,allDemPixelsAudited=True,auditedMaxError=0,auditedErrorTarget=error);write(dst/'packet.json',out);audit=dict(empty=True,validPixels=0,max=0,errorTarget=error,fullTile=True);write(dst/'selection.json',audit);return audit
 original=np.load(src/'control-gradient.npz')['points'];initial=ROOT/f'build/natural-raw-c1-128-quarter-v1/{region}/{(error if source_error is None else source_error):g}/{x}-{y}/control-gradient.npz'
 support_count=0
 if (dst/'fit-state.npz').exists():
  with np.load(dst/'fit-state.npz') as state:p=state['points'].copy();support_count=int(state['support_count']) if 'support_count' in state else 0
 elif seed_policy=='dem-corners':
  def blocks():
   for tx,ty in keys:
    x0=max(int(lo[0]),tx*256);x1=min(int(hi[0])+1,(tx+1)*256);y0=max(int(lo[1]),ty*256);y1=min(int(hi[1])+1,(ty+1)*256)
    a=tile(tx,ty)[y0-ty*256:y1-ty*256,x0-tx*256:x1-tx*256];yy,xx=np.nonzero(np.isfinite(a))
    yield np.column_stack(((xx+x0+.5)*STEP-world[0],(yy+y0+.5)*STEP-world[1])),a[yy,xx]
  guards=original[~np.all(abs(original[:,:2])<=side/2+halo+1,axis=1)]
  if len(guards)!=4:raise ValueError('外周ガード点が4点ではありません')
  p=np.vstack((guards,corner_support(blocks(),side,world)))
 elif initial.exists():
  try:p=np.load(initial)['points'].copy()
  except (OSError,ValueError):p=original.copy()
 else:
  real=np.all(abs(original[:,:2])<=side/2+halo+1,axis=1);p=np.vstack((original[~real],original[real][::4]))
 reference=DemReference(world,half,keys=keys,read=tile,step=STEP)
 initial_count=len(p)-4
 def status(**extra):
  write(dst/'progress.json',dict(stage='stream-full-dem-audit',region=region,error=error,x=x,y=y,points=len(p)-4,seconds=time.monotonic()-started,**extra))
 for iteration in range(max_iterations):
  model=AuxiliarySibsonSurface(p);todo=sorted(dirty,key=lambda k:(k[1],k[0]));dirty.clear();status(iteration=iteration,blocksDone=0,blocksTotal=len(todo),finalAudit=final_audit)
  for block_i,(tx,ty) in enumerate(todo):
   q,target=reference.block((tx,ty))
   if len(q):
    predicted=model(q);residual=abs(predicted-target);core=np.all(abs(q)<=side/2,axis=1);bad=np.flatnonzero(residual>error);order=bad[np.argsort(residual[bad])[::-1]][:128];candidates=np.column_stack((q[order],target[order],residual[order]));cache[(tx,ty)]=dict(max=float(residual.max()),sumsq=float(residual@residual),count=len(q),coreCount=int(core.sum()),coreMax=float(residual[core].max()) if core.any() else 0,candidates=candidates)
   else:cache[(tx,ty)]=dict(max=0,sumsq=0,count=0,coreCount=0,coreMax=0,candidates=np.empty((0,4)))
   if block_i%16==0 or block_i+1==len(todo):status(iteration=iteration,blocksDone=block_i+1,blocksTotal=len(todo),finalAudit=final_audit,auditedMaximum=max(v['max'] for v in cache.values()))
  maximum=max(v['max'] for v in cache.values());valid_pixels=sum(v['count'] for v in cache.values());history.append(dict(iteration=iteration,points=len(p)-4,max=maximum,blocks=len(todo),finalAudit=final_audit))
  if maximum<=error:
   if final_audit:break
   # 局所再評価の取りこぼしが無いことを、最後に全PNG区画を再走査して確認。
   dirty=set(keys);final_audit=True;continue
  final_audit=False;all_candidates=np.vstack([v['candidates'] for v in cache.values()]);order=np.argsort(all_candidates[:,3])[::-1];known=set(map(tuple,np.rint((p[:,:2]+world)/.0625).astype(np.int64)));chosen=[];added=[];source_keys=[]
  for i in order:
   q=all_candidates[i,:2];qx=float(q[0]);qy=float(q[1]);quant=np.rint((q+world)/.0625).astype(np.int64);key=tuple(quant)
   if key in known or any((qx-a[0])*(qx-a[0])+(qy-a[1])*(qy-a[1])<4096 for a in chosen):continue
   chosen.append((qx,qy));added.append([*(quant*.0625-world),round(float(all_candidates[i,2])/.125)*.125]);known.add(key);source_keys.append(tuple(np.floor((q+world)/(STEP*256)).astype(int)))
   if len(added)==batch_size:break
  if not added:
   # 量子化済み点の直近で残差が残る場合、低残差の隣接実DEM点も
   # 支持点として補う。値を推測せず元DEMから取り、必ず全域再監査する。
   for i in order[:128]:
    q=all_candidates[i,:2];center=np.floor((q+world)/STEP).astype(int)
    for radius in [1,2,4]:
     for dx,dy in [(1,0),(-1,0),(0,1),(0,-1),(1,1),(1,-1),(-1,1),(-1,-1)]:
      ix=int(center[0]+dx*radius);iy=int(center[1]+dy*radius);height=tile(ix//256,iy//256)[iy%256,ix%256]
      if not np.isfinite(height):continue
      location=(np.asarray([ix,iy])+.5)*STEP;quant=np.rint(location/.0625).astype(np.int64);key=tuple(quant)
      if key in known:continue
      local=quant*.0625-world
      if np.any(abs(local)>side/2+halo):continue
      added.append([*local,round(float(height)/.125)*.125]);known.add(key);source_keys.append((ix//256,iy//256));support_count+=1
      if len(added)>=batch_size:break
     if len(added)>=batch_size:break
    if len(added)>=batch_size:break
   if not added:raise ValueError('全域監査で誤差超過、未登録の追加点も近傍実DEM支持点も無い')
  boxes=influence(model,added)
  for tx,ty in keys:
   b=[tx*256*STEP-world[0],ty*256*STEP-world[1],(tx+1)*256*STEP-world[0],(ty+1)*256*STEP-world[1]]
   if any(b[0]<=v[2] and b[2]>=v[0] and b[1]<=v[3] and b[3]>=v[1] for v in boxes):dirty.add((tx,ty))
  dirty.update(k for k in source_keys if k in cache);p=np.vstack((p,added));save_points(dst/'fit-state.npz',p,support_count)
  if len(p)>20000:raise ValueError('安全上限20000点を超えたため未完了')
 else:raise ValueError('反復上限到達、未完了')
 status(stageDetail='fit-C1-and-pack',iteration=iteration,blocksDone=len(keys),blocksTotal=len(keys),finalAudit=True,auditedMaximum=maximum)
 audit=dict(seedPolicy=seed_policy,insertionBatch=batch_size,errorTarget=error,fullTile=True,stabilitySupportPoints=support_count,validPixels=valid_pixels,corePixels=sum(v['coreCount'] for v in cache.values()),max=maximum,coreMax=max(v['coreMax'] for v in cache.values()),rmse=math.sqrt(sum(v['sumsq'] for v in cache.values())/valid_pixels) if valid_pixels else 0,initialPoints=initial_count,finalPoints=len(p)-4,originalPoints=len(original)-4,history=history,seconds=time.monotonic()-started)
 if controls_only:
  # 中間曲面は削減用の制御点だけを保存し、GPU用の補助情報は最終形状でのみ作る。
  save_points(dst/'control-gradient.npz',p,support_count);audit['controlsOnly']=True;write(dst/'selection.json',audit);return audit
 bounds=np.asarray(info['bounds']);n=max(16384,min(262144,2**int(np.ceil(np.log2(len(p)*4)))));queries=qmc.Sobol(2,scramble=True,seed=9124).random_base2(int(np.log2(n)))*(bounds[2:]-bounds[:2])+bounds[:2];c1=GlobalC1(p,strength=.01,queries=queries,surface=model);close_cells(c1.fast_model)
 meta={k:info[k] for k in ['region','error','x','y','side','origin','coreBounds','worldFieldBounds','halo']};meta.update(curve='Sibson-error-driven-full-tile-dem-pixels',lpf=False,sourceGeometryBytes=len(p)*12,maxErrorGuaranteed=False,allDemPixelsAudited=True,auditedErrorTarget=error,auditedMaxError=maximum,auditWorldBounds=[*(world-half),*(world+half)],auditScope='all valid original DEM pixel centers in full tile plus 250m seam band; usual Sibson only; excludes pixel gaps, C1, rendered grid and blending')
 packet,out=pack(p,c1.gradient,bounds,meta,model=c1.fast_model,max_candidates=48,max_depth=pack_depth)
 if out['unsafeLeaves']:raise ValueError('unsafe GPU Voronoi leaves')
 temporary=dst/'packet.tmp.bin';temporary.write_bytes(packet);temporary.replace(dst/'packet.bin');temp=dst/'control-gradient.tmp.npz';np.savez_compressed(temp,points=p,gradient=c1.gradient);temp.replace(dst/'control-gradient.npz');out['errorFitAudit']=audit;write(dst/'packet.json',out);write(dst/'selection.json',audit);status(stageDetail='complete',iteration=iteration,blocksDone=len(keys),blocksTotal=len(keys),finalAudit=True,auditedMaximum=maximum);return audit
