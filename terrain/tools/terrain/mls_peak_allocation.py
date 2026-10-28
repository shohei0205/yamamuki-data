"""富士山2048点の山頂重点配分を、同じMLS復元条件で比較する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import json,sqlite3,time,zlib
from pathlib import Path
import numpy as np
from scipy.ndimage import gaussian_filter
from scipy.sparse.linalg import lsmr
from scipy.stats import qmc,norm
from scipy.spatial import cKDTree
from free_mls import LocalSupport,operator,reference_values,SPAN
from adaptive_allocation_surface import distribute

def peak_density(original,narrow_mass,count=2048):
    axis=np.linspace(-SPAN/2,SPAN/2,257);x,y=np.meshgrid(axis,axis);xy=np.column_stack((x.ravel(),y.ravel()));h=reference_values(original,xy).reshape(257,257)
    curvature=np.abs(np.gradient(np.gradient(h,axis=0),axis=0))+np.abs(np.gradient(np.gradient(h,axis=1),axis=1))
    base=np.sqrt(gaussian_filter(curvature**2,max(1,(SPAN/np.sqrt(count))/125)));base=np.clip(.35+.65*base/max(base.mean(),1e-9),.25,3.);base/=base.sum()
    narrow=np.exp(-(x*x+y*y)/(2*800**2));narrow/=narrow.sum();wide=np.exp(-(x*x+y*y)/(2*2400**2));wide/=wide.sum()
    density=(.95-narrow_mass)*base+narrow_mass*narrow+.05*wide
    return density/density.mean()

def fit(points,original):
    support=LocalSupport(points,pre_smooth=False,floor_factor=.5)
    global_train=(qmc.Sobol(2,scramble=True,seed=199).random_base2(14)-.5)*SPAN
    focus_train=np.clip(norm.ppf(qmc.Sobol(2,scramble=True,seed=451).random_base2(13)), -5,5)*1500
    train=np.vstack((global_train,focus_train));truth=reference_values(original,train)
    valid=(qmc.Sobol(2,scramble=True,seed=682).random_base2(12)-.5)*SPAN
    focus_valid=np.clip(norm.ppf(qmc.Sobol(2,scramble=True,seed=881).random_base2(12)),-5,5)*1000
    av=operator(points,valid,support,curvature=1e-5);ap=operator(points,focus_valid,support,curvature=1e-5);a=operator(points,train,support,curvature=1e-5)
    initial=reference_values(original,points);r=lsmr(a,truth-a@initial,damp=.15,atol=1e-5,btol=1e-5,maxiter=800)
    if r[1] not in (0,1,2,4,5):raise ValueError(f'近似が収束しません: {r[1]}')
    z=np.asarray(initial+r[0],'<f4');e=av@z-reference_values(original,valid);ep=ap@z-reference_values(original,focus_valid)
    peak_error=float(abs((operator(points,np.array([[0.,0.]]),support,curvature=1e-5)@z)[0]-reference_values(original,np.array([[0.,0.]]))[0]))
    separation=cKDTree(points).query(points,k=2)[0][:,1];radii=support(valid)
    return z,dict(rmse=float(np.sqrt(np.mean(e**2))),maxError=float(abs(e).max()),peakAreaRmse=float(np.sqrt(np.mean(ep**2))),peakAreaMax=float(abs(ep).max()),peakError=peak_error,within500=int((np.linalg.norm(points,axis=1)<=500).sum()),within1000=int((np.linalg.norm(points,axis=1)<=1000).sum()),within2000=int((np.linalg.norm(points,axis=1)<=2000).sum()),minimumSeparation=float(separation.min()),radiusMeters=float(np.median(radii)),radiusMin=float(radii.min()),radiusMax=float(radii.max()),refitIterations=int(r[2]))

def main():
    with sqlite3.connect('build/free-mls-v3/mls.sqlite3') as db:raw,b=db.execute("SELECT info,points FROM surfaces WHERE region='fuji' AND budget=2048").fetchone()
    old_info=json.loads(raw);old_points=np.frombuffer(zlib.decompress(b),'<f4')[1:].reshape(-1,3)[:,:2]
    with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:b=db.execute("SELECT coefficients FROM surfaces WHERE region='fuji' AND n=256").fetchone()[0]
    original=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261);rows=[]
    for mass in [0.,.10,.20,.30]:
        points=old_points if mass==0 else distribute(peak_density(original,mass),2048,SPAN/np.sqrt(2048)).astype('<f4')
        z,measure=fit(points,original);measure['narrowMass']=mass;rows.append(measure);print(json.dumps(measure),flush=True)
        folder=Path('build')/('free-mls-fuji-local-v1' if mass==0 else f'free-mls-fuji-peak{round(mass*100)}-v1');folder.mkdir(exist_ok=True)
        path=folder/'mls.sqlite3'
        if path.exists():raise ValueError('既存結果を保持します')
        packed=zlib.compress(np.array([1.25],'<f4').tobytes()+np.column_stack((points,z)).astype('<f4').tobytes(),6)
        info=dict(old_info);info.update(measure);info.update(kernel='MLS-quadratic-local-support-v1',supportMode='local-C2-v1',curvaturePenalty=1e-5,supportFloorFactor=.5,radiusHeaderName='neighborRadiusScale',storedBytes=len(packed),peakAllocation=mass>0,peakNarrowSigma=800,peakWideSigma=2400,peakWideMass=.05)
        with sqlite3.connect(path) as db:db.execute('CREATE TABLE surfaces(region TEXT,budget INTEGER,info TEXT,points BLOB,PRIMARY KEY(region,budget)) WITHOUT ROWID');db.execute('INSERT INTO surfaces VALUES(?,?,?,?)',('fuji',2048,json.dumps(info),packed));db.commit()
    Path('build/mls-fuji-peak-comparison.json').write_text(json.dumps(rows,indent=2),encoding='utf-8')
if __name__=='__main__':main()
