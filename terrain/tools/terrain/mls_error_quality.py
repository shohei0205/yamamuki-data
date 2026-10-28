"""誤差条件に応じて点数と配分を探索する独立MLS品質レベル。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import argparse,json,sqlite3,time,zlib
from pathlib import Path
import numpy as np
from scipy.stats import qmc,norm
from scipy.sparse.linalg import lsmr
from free_mls import LocalSupport,operator,reference_values,SPAN
from mls_peak_allocation import peak_density
from adaptive_allocation_surface import distribute

def peak_samples(power,seed,sigma):return np.clip(norm.ppf(qmc.Sobol(2,scramble=True,seed=seed).random_base2(power)),-5,5)*sigma

def requirements(error,xy,tolerance,peak_ratio=.5):
    e=abs(np.asarray(error));mask=np.linalg.norm(xy,axis=1)<=2000;maximum=float(e.max());peak=float(e[mask].max()) if mask.any() else 0.
    return dict(measuredMaxError=maximum,measuredPeakMaxError=peak,measuredRmse=float(np.sqrt(np.mean(e*e))),converged=bool(maximum<=tolerance and peak<=tolerance*peak_ratio))

def next_count(count,maximum,measurement,tolerance):
    if count>=maximum:return count
    ratio=max(measurement['measuredMaxError']/tolerance,measurement['measuredPeakMaxError']/(tolerance*.5))
    return min(maximum,max(count+16,int(np.ceil(count*np.clip(ratio**.75,1.15,1.8)))))

def allocate(original,count):
    points=distribute(peak_density(original,.20,count=count),count,SPAN/np.sqrt(count)).astype('<f4')
    # 領域外への外挿を避ける幾何学的な四隅。内部は格子へ固定しない。
    used=set()
    for corner in [[-16000,-16000],[-16000,16000],[16000,-16000],[16000,16000]]:
        order=np.argsort(np.linalg.norm(points-corner,axis=1));i=next(int(i) for i in order if int(i) not in used);points[i]=corner;used.add(i)
    return points

def fit_model(original,points,tolerance):
    support=LocalSupport(points,pre_smooth=False,floor_factor=.5,grid_n=128)
    axis=np.linspace(-SPAN/2,SPAN/2,257);xx,yy=np.meshgrid(axis,axis)
    train=np.vstack((np.column_stack((xx.ravel(),yy.ravel())),peak_samples(13,452,1500),points))
    truth=reference_values(original,train);initial=reference_values(original,points);a=operator(points,train,support,curvature=1e-5)
    weight=np.ones(len(train));z=initial.copy();iterations=0;best=None
    local_tolerance=np.where(np.linalg.norm(train,axis=1)<=2000,tolerance*.5,tolerance)
    for _ in range(5):
        root=np.sqrt(weight);r=lsmr(a.multiply(root[:,None]).tocsr(),(truth-a@initial)*root,damp=.15,atol=1e-5,btol=1e-5,maxiter=1000)
        if r[1] not in (0,1,2,4,5):raise ValueError(f'MLSの再適合が収束しません: {r[1]}')
        candidate=initial+r[0];iterations+=int(r[2]);errors=abs(a@candidate-truth);score=float(np.max(errors/local_tolerance))
        if best is None or score<best[0]:best=(score,candidate.copy())
        weight=np.maximum(weight,np.clip((errors/np.maximum(local_tolerance*.5,1e-6))**2,1,256))
    return np.asarray(best[1],'<f4'),support,iterations

def generate(a):
    if a.minimum<16 or a.maximum<a.minimum or not a.errors or any(not np.isfinite(v) or v<=0 for v in a.errors):raise ValueError('点数の範囲と正の許容誤差を指定してください')
    a.output.mkdir(parents=True,exist_ok=True);path=a.output/'quality.sqlite3'
    if path.exists():raise ValueError('既存結果を残すため新しい出力先を指定してください')
    axis=np.linspace(-SPAN/2,SPAN/2,257);xx,yy=np.meshgrid(axis,axis)
    audit=np.vstack((np.column_stack((xx.ravel(),yy.ravel())),(qmc.Sobol(2,scramble=True,seed=904).random_base2(14)-.5)*SPAN,peak_samples(12,935,1000)))
    with sqlite3.connect(a.input) as source,sqlite3.connect(path) as db:
        db.execute('CREATE TABLE levels(region TEXT,target_error REAL,info TEXT,points BLOB,PRIMARY KEY(region,target_error)) WITHOUT ROWID');report=[]
        for region in a.regions:
            raw,b=source.execute('SELECT info,coefficients FROM surfaces WHERE region=? AND n=256',(region,)).fetchone();base=json.loads(raw);original=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261)
            truth=reference_values(original,audit);count=a.minimum;history=[];previous=None
            for tolerance in sorted(set(a.errors),reverse=True):
                started=time.monotonic();trials=0
                if previous is not None and requirements(previous[3],audit,tolerance)['converged']:
                    points,z,support,errors=previous;measurement=requirements(errors,audit,tolerance)
                else:
                    while True:
                        points=allocate(original,count);z,support,iterations=fit_model(original,points,tolerance);errors=operator(points,audit,support,curvature=1e-5)@z-truth;measurement=requirements(errors,audit,tolerance);trials+=1
                        progress=dict(region=region,targetError=tolerance,controls=count,measurement=measurement,maximumPoints=a.maximum,completedLevels=len(report));history.append(progress);(a.output/'progress.json').write_text(json.dumps(progress,indent=2),encoding='utf-8');print(json.dumps(progress),flush=True)
                        if measurement['converged'] or count>=a.maximum:break
                        count=next_count(count,a.maximum,measurement,tolerance)
                previous=(points,z,support,errors);count=len(points);packed=zlib.compress(np.array([1.25],'<f4').tobytes()+np.column_stack((points,z)).astype('<f4').tobytes(),6);radii=support(audit[::16])
                info=dict(region=region,controls=count,budget=count,qualityMode=True,targetError=tolerance,targetPeakError=tolerance*.5,maximumPoints=a.maximum,minimumPoints=a.minimum,stopReason='tolerance-met' if measurement['converged'] else 'maximum-points',**measurement,mls=True,kernel='MLS-error-driven-local-v1',supportMode='local-C2-v1',supportFloorFactor=.5,supportGridN=128,curvaturePenalty=1e-5,radiusHeaderName='neighborRadiusScale',radiusMeters=float(np.median(radii)),radiusMin=float(radii.min()),radiusMax=float(radii.max()),spacingMeters=SPAN/np.sqrt(count),storedBytes=len(packed),rawBytes=4+count*12,originalBytes=len(b),rmse=measurement['measuredRmse'],maxError=measurement['measuredMaxError'],peakError=float(abs((operator(points,np.array([[0.,0.]]),support,curvature=1e-5)@z)[0]-reference_values(original,np.array([[0.,0.]]))[0])),summit=base['summit'],auditCount=len(audit),auditGridStepMeters=125,independentCheckCount=16384+4096,source='saved-B-spline',searchTrials=trials,boundaryAnchors=4,seconds=time.monotonic()-started)
                db.execute('INSERT INTO levels VALUES(?,?,?,?)',(region,tolerance,json.dumps(info),packed));db.commit();report.append(info)
                (a.output/'report.json').write_text(json.dumps(report,indent=2),encoding='utf-8');(a.output/'history.json').write_text(json.dumps(history,indent=2),encoding='utf-8');print('LEVEL',json.dumps(info),flush=True)
    (a.output/'progress.json').write_text(json.dumps(dict(stage='complete',levels=len(report)),indent=2),encoding='utf-8')
if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--input',type=Path,default=Path('build/ridge-spline-prototype-v4/spline.sqlite3'));p.add_argument('--output',type=Path,default=Path('build/mls-error-quality-v7'));p.add_argument('--regions',nargs='+',default=['fuji']);p.add_argument('--errors',nargs='+',type=float,default=[200,100,50,25]);p.add_argument('--minimum',type=int,default=256);p.add_argument('--maximum',type=int,default=32768);a=p.parse_args();generate(a)
