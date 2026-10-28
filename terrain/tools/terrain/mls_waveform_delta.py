"""MLSの曲面間残差をDCT量子化して保存する比較。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import json,sqlite3,zlib,lzma,struct,time
from pathlib import Path
import numpy as np
from scipy.fft import dctn,idctn
from scipy.ndimage import map_coordinates,spline_filter
from scipy.stats import qmc
from free_mls import LocalSupport,operator,reference_values,SPAN
from mls_error_quality import peak_samples,requirements
from mls_quant_codec import decode_quant
HEADER=struct.Struct('<4sHff')
def encode(grid,step):
 grid=np.asarray(grid,float)
 if grid.ndim!=2 or grid.shape[0]!=grid.shape[1] or not 2<=len(grid)<=65535 or step<=0:raise ValueError('不正な格子または刻み')
 c=dctn(grid,type=2,norm='ortho');v=np.rint(c/step)
 if not np.isfinite(v).all() or np.max(abs(v))>np.iinfo(np.int32).max:raise ValueError('係数の範囲外')
 v=v.astype('<i4');raw=v.view('u1').reshape(-1,4).T.copy().tobytes()
 return HEADER.pack(b'WFD1',len(grid),step,SPAN)+lzma.compress(raw,preset=6)
def decode(blob):
 if len(blob)<HEADER.size:raise ValueError('ヘッダーが不足')
 magic,n,step,span=HEADER.unpack_from(blob)
 if magic!=b'WFD1' or n<2 or step<=0 or span!=SPAN:raise ValueError('形式が不正')
 raw=lzma.decompress(blob[HEADER.size:])
 if len(raw)!=n*n*4:raise ValueError('係数の長さが不正')
 c=np.frombuffer(raw,'u1').reshape(4,n*n).T.copy().view('<i4').reshape(n,n)
 return idctn(c.astype(float)*step,type=2,norm='ortho')
def sample(grid,xy):
 n=len(grid);coords=(np.asarray(xy)+SPAN/2)/SPAN*(n-1)
 return map_coordinates(spline_filter(grid,order=3,mode='mirror'),[coords[:,1],coords[:,0]],order=3,mode='mirror',prefilter=False)
def evaluate(points,query):
 return operator(points[:,:2],query,LocalSupport(points[:,:2],pre_smooth=False,floor_factor=.5),curvature=1e-5)@points[:,2]
def main():
 out=Path('build/mls-waveform-delta-v2');out.mkdir(exist_ok=True);path=out/'quality.sqlite3'
 if path.exists():raise ValueError('既存出力は保持する')
 axis=np.linspace(-16000,16000,257);x,y=np.meshgrid(axis,axis)
 q=np.vstack((np.column_stack((x.ravel(),y.ravel())),(qmc.Sobol(2,scramble=True,seed=904).random_base2(14)-.5)*SPAN,peak_samples(12,935,1000)))
 h=np.vstack(((qmc.Sobol(2,scramble=True,seed=5904).random_base2(15)-.5)*SPAN,peak_samples(13,5935,1000)))
 with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:b=db.execute("SELECT coefficients FROM surfaces WHERE region='fuji' AND n=256").fetchone()[0]
 original=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261);truth=reference_values(original,q);ht=reference_values(original,h);models={};blobs={}
 with sqlite3.connect('build/mls-hybrid-selected-v2/quality.sqlite3') as db:
  for e,parent,b in db.execute('SELECT error,parent_error,points FROM levels ORDER BY error DESC'):
   p=decode_quant(b);models[e]=np.vstack((models[parent],p)) if parent else p;blobs[e]=b
 trials=[];selected=[];comparisons=[]
 with sqlite3.connect(path) as dest:
  dest.execute('CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT) WITHOUT ROWID');dest.execute('CREATE TABLE levels(error REAL PRIMARY KEY,parent_error REAL,codec TEXT,info TEXT,blob BLOB)')
  params=dict(region='fuji',span=SPAN,scale=1.25,supportFloorFactor=.5,supportGridN=128,preSmooth=False,curvaturePenalty=1e-5,waveInterpolation='cubic-mirror',maxParentDepth=1)
  for k,v in params.items():dest.execute('INSERT INTO metadata VALUES(?,?)',(k,json.dumps(v)))
  for parent,e in ((200,100),(50,25)):
   started=time.monotonic();pq=evaluate(models[parent],q);ph=evaluate(models[parent],h);target=evaluate(models[e],q);targeth=evaluate(models[e],h);candidates=[]
   for n in (257,513):
    axis=np.linspace(-16000,16000,n);x,y=np.meshgrid(axis,axis);gridq=np.column_stack((x.ravel(),y.ravel()));cache=out/f'grids-E{e}-n{n}.npz'
    old=Path('build/mls-waveform-delta-v1')/cache.name
    if not cache.exists() and old.exists():cache=old
    if cache.exists():
     with np.load(cache) as f:child=f['child'];coarse=f['coarse']
    else:
     child=evaluate(models[e],gridq).reshape(n,n);coarse=evaluate(models[parent],gridq).reshape(n,n);np.savez(cache,child=child,coarse=coarse)
    for mode,g in (('residual',child-coarse),('independent-wave',child)):
     for step in (.25,1,4,16,64):
      b=encode(g,step);restored=decode(b);value=sample(restored,q)+(pq if mode=='residual' else 0);distortion=value-target;measurement=requirements(value-truth,q,e)
      item=dict(error=e,parent=parent,mode=mode,n=n,step=step,bytes=len(b),nonzero=int(np.count_nonzero(np.rint(dctn(g,type=2,norm='ortho')/step))),surfaceMaxDistortion=float(abs(distortion).max()),surfaceRmse=float(np.sqrt(np.mean(distortion**2))),audit=measurement)
      trials.append(item)
      if measurement['converged']:candidates.append((len(b),item,b))
    print(json.dumps(dict(stage='grid-complete',error=e,n=n,seconds=time.monotonic()-started)),flush=True)
   # 探索に使わなかった点で候補を容量順に検証する。失敗は保持して次を試す。
   best={}
   for size,item,b in sorted(candidates,key=lambda v:v[0]):
    mode=item['mode']
    if mode in best:continue
    hv=sample(decode(b),h)+(ph if mode=='residual' else 0);item['heldout']=requirements(hv-ht,h,e);item['heldoutSurfaceMaxDistortion']=float(abs(hv-targeth).max())
    if item['heldout']['converged']:best[mode]=(item,b)
   if 'residual' not in best:raise ValueError('差分波形に条件を満たす候補がありません')
   item,b=best['residual'];selected.append(item)
   anchorInfo=dict(controls=len(models[parent]),bytes=len(blobs[parent]))
   dest.execute('INSERT INTO levels VALUES(?,?,?,?,?)',(parent,None,'MLQ1',json.dumps(anchorInfo),blobs[parent]));dest.execute('INSERT INTO levels VALUES(?,?,?,?,?)',(e,parent,'WFD1',json.dumps(item),b));dest.commit()
   print('SELECTED',json.dumps({k:v[0] for k,v in best.items()}),flush=True)
   comparisons.extend(v[0] for v in best.values())
   (out/'trials.json').write_text(json.dumps(trials,indent=2),encoding='utf-8');(out/'comparison.json').write_text(json.dumps(comparisons,indent=2),encoding='utf-8')
 result=dict(levels=selected,blobBytes=sum(v['bytes'] for v in selected)+len(blobs[200])+len(blobs[50]),xyzDeltaBlobBytes=sum(map(len,blobs.values())),sqliteBytes=path.stat().st_size,validationPoints=len(h))
 (out/'report.json').write_text(json.dumps(result,indent=2),encoding='utf-8');print('RESULT',json.dumps(result),flush=True)
if __name__=='__main__':main()
