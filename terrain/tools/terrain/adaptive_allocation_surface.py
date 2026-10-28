"""共通LPFを維持し、再現誤差によって中心を再配分する自由曲面。"""
import argparse
import json
import sqlite3
import time
import zlib
from pathlib import Path
import numpy as np
from scipy.ndimage import gaussian_filter,map_coordinates
from scipy.spatial import cKDTree
from scipy.stats import qmc
from lpf_curve_compare import Source
from scale_surface import SPAN,BUDGETS,terms,evaluate


def sample_density(density,count,seed):
    """面積密度の逆CDFへSobol列を写し、格子へ固定せず配置する。"""
    u=qmc.Sobol(2,scramble=True,seed=seed).random_base2(int(np.ceil(np.log2(count))))[:count]
    mass=density.sum(axis=1);cdf=np.r_[0,np.cumsum(mass)];cdf/=cdf[-1]
    row=np.clip(np.searchsorted(cdf,u[:,1],side='right')-1,0,len(mass)-1)
    fy=(u[:,1]-cdf[row])/np.maximum(cdf[row+1]-cdf[row],1e-20)
    out=np.empty((count,2));out[:,1]=(row+fy)/density.shape[0]*SPAN-SPAN/2
    for y in np.unique(row):
        ids=np.flatnonzero(row==y);cx=np.r_[0,np.cumsum(density[y])];cx/=cx[-1]
        col=np.clip(np.searchsorted(cx,u[ids,0],side='right')-1,0,density.shape[1]-1)
        fx=(u[ids,0]-cx[col])/np.maximum(cx[col+1]-cx[col],1e-20)
        out[ids,0]=(col+fx)/density.shape[1]*SPAN-SPAN/2
    return out


def distribute(density,count,spacing):
    centers=sample_density(density,count,2841)
    candidates=sample_density(density,max(65536,count*4),7183)
    for _ in range(2):
        labels=cKDTree(centers).query(candidates)[1];mass=np.bincount(labels,minlength=count)
        mean=np.column_stack([np.bincount(labels,weights=candidates[:,k],minlength=count) for k in range(2)])/np.maximum(mass[:,None],1)
        move=mean-centers;move[mass==0]=0;length=np.linalg.norm(move,axis=1)
        move*=np.minimum(1.,spacing*.25/np.maximum(length,1e-20))[:,None]
        centers=np.clip(centers+move,-SPAN/2,SPAN/2)
    return centers


def radii_for_centers(centers,spacing,cover):
    tree=cKDTree(centers);dist=tree.query(centers,k=9)[0][:,-1]
    # LPFの尺度より小さな局所面を作らない。平坦部では半径を広げられる。
    minimum=spacing*.75
    radii=np.maximum(dist,minimum)
    distance,labels=tree.query(cover);required=np.zeros(len(centers));np.maximum.at(required,labels,distance)
    # 125m格子を覆うだけでなく、格子間にも到達する余裕を持つ。
    radii=np.maximum(radii,1.2*(required+125/np.sqrt(2)))
    return radii,minimum


def fit_sampled(reference,origin,step,centers,radii,minimum):
    nodes=qmc.Sobol(2,scramble=True,seed=1709).random_base2(8)
    theta=nodes[:,0]*2*np.pi;length=np.sqrt(nodes[:,1]);q=np.column_stack((np.cos(theta)*length,np.sin(theta)*length))
    a=terms(q);w=np.exp(-4*np.sum(q*q,axis=1));out=np.empty((len(centers),10),'<f4')
    for start in range(0,len(centers),256):
        c=centers[start:start+256];r=radii[start:start+256];xy=c[:,None,:]+r[:,None,None]*q
        coordinates=(xy-origin)/step
        valid=np.all((coordinates>=0)&(coordinates<np.asarray(reference.shape)[::-1]-1),axis=2)
        height=map_coordinates(reference,coordinates.reshape(-1,2).T[[1,0]],order=1,prefilter=False,mode='nearest').reshape(len(c),-1)
        weight=valid*w;normal=np.einsum('nk,kp,kq->npq',weight,a,a,optimize=True)
        rhs=np.einsum('nk,kp->np',weight*height,a,optimize=True)
        normal+=np.diag([1e-12,1e-10,1e-10,1e-8,1e-8,1e-8])
        coefficients=np.linalg.solve(normal,rhs[...,None])[...,0]
        confidence=(minimum/r)**2
        out[start:start+len(c)]=np.column_stack((c,r,coefficients,confidence))
    return out


def optimize(target,origin,step,uniform,count,iterations=3):
    spacing=SPAN/np.sqrt(count);sigma=spacing*.3
    axis=np.linspace(-SPAN/2,SPAN/2,257);x,y=np.meshgrid(axis,axis);query=np.column_stack((x.ravel(),y.ravel()))
    truth=map_coordinates(target,((query-origin)/step).T[[1,0]],order=1,prefilter=False)
    validation=(qmc.Sobol(2,scramble=True,seed=8911).random_base2(14)-.5)*SPAN
    valid_truth=map_coordinates(target,((validation-origin)/step).T[[1,0]],order=1,prefilter=False)
    baseline=np.column_stack((uniform,np.ones(count))).astype('<f4')
    baseline_valid=float(np.sqrt(np.mean((evaluate(baseline,validation)-valid_truth)**2)))
    best=baseline;score=baseline_valid;density=np.ones((257,257));history=[]
    for iteration in range(iterations):
        error=(evaluate(best,query)-truth).reshape(257,257)
        rms=np.sqrt(gaussian_filter(error*error,max(.5,sigma/125),mode='nearest'))
        factor=np.sqrt((rms+.01)/(np.mean(rms)+.01))
        proposed=np.clip(density*factor,.08,8.);proposed/=proposed.mean()
        density=.5*density+.5*proposed;density/=density.mean()
        centers=distribute(density,count,spacing);radii,minimum=radii_for_centers(centers,spacing,query)
        candidate=fit_sampled(target,origin,step,centers,radii,minimum)
        candidate_score=float(np.sqrt(np.mean((evaluate(candidate,validation)-valid_truth)**2)))
        accepted=candidate_score<score
        if accepted:best=candidate;score=candidate_score
        history.append(dict(iteration=iteration+1,validationRmse=candidate_score,accepted=accepted,radiusMin=float(radii.min()),radiusMedian=float(np.median(radii)),radiusMax=float(radii.max())))
        print('再配分',count,iteration+1,'検証RMSE',candidate_score,'採用',accepted,flush=True)
    return best,dict(initialValidationRmse=baseline_valid,validationRmse=score,allocationHistory=history,allocationIterations=sum(h['accepted'] for h in history))


def generate(args):
    args.output.mkdir(parents=True,exist_ok=True);path=args.output/'adaptive.sqlite3'
    if path.exists():raise ValueError('既存結果を残すため新しい出力先を指定してください')
    source=Source(args.source);step=SPAN/1024;origin=-SPAN/2-160*step;axis=np.arange(1345)*step+origin;x,y=np.meshgrid(axis,axis)
    with sqlite3.connect(args.regions.resolve().as_uri()+'?mode=ro',uri=True) as meta,sqlite3.connect(args.uniform.resolve().as_uri()+'?mode=ro',uri=True) as uniform_db,sqlite3.connect(path) as db:
        db.execute('CREATE TABLE surfaces(region TEXT,budget INTEGER,info TEXT,model BLOB,PRIMARY KEY(region,budget)) WITHOUT ROWID');report=[]
        for region,raw in meta.execute('SELECT name,info FROM regions ORDER BY name'):
            if args.region and args.region!=region:continue
            center=np.asarray(json.loads(raw)['center']);reference=source.sample(x+center[0],y+center[1]);start=time.monotonic()
            qa=np.linspace(-SPAN/2,SPAN/2,257);qx,qy=np.meshgrid(qa,qa);query=np.column_stack((qx.ravel(),qy.ravel()))
            original=map_coordinates(reference,((query-origin)/step).T[[1,0]],order=1,prefilter=False)
            for count in args.budgets:
                raw_info,blob=uniform_db.execute('SELECT info,model FROM surfaces WHERE region=? AND budget=?',(region,count)).fetchone();base_info=json.loads(raw_info)
                uniform=np.frombuffer(zlib.decompress(blob),'<f4').reshape(-1,9)
                target=gaussian_filter(reference,base_info['sigmaMeters']/step,mode='nearest',truncate=4.)
                print(region,count,'誤差に応じて中心を再配分',flush=True)
                patches,detail=optimize(target,origin,step,uniform,count,args.iterations)
                restored=evaluate(patches,query);truth=map_coordinates(target,((query-origin)/step).T[[1,0]],order=1,prefilter=False)
                tree=cKDTree(patches[:,:2]);separation=tree.query(patches[:,:2],k=2)[0][:,1];density=tree.query_ball_point(patches[:,:2],base_info['spacingMeters']*2,return_length=True)
                packed=zlib.compress(patches.tobytes(),6);summit=float(evaluate(patches,[[0,0]])[0])
                info=dict(base_info);info.update(detail);info.update(kernel='Adaptive-scale-C2-local-quadratic',storedBytes=len(packed),rawBytes=patches.nbytes,filteredRmse=float(np.sqrt(np.mean((restored-truth)**2))),filteredMaxError=float(np.max(abs(restored-truth))),rmse=float(np.sqrt(np.mean((restored-original)**2))),maxError=float(np.max(abs(restored-original))),summit=[0.,0.,summit],peakError=float(abs(summit-source.sample(center[0],center[1]).item())),radiusMin=float(patches[:,2].min()),radiusMedian=float(np.median(patches[:,2])),radiusMax=float(patches[:,2].max()),radiusFloor=SPAN/np.sqrt(count)*.75,densityP50=float(np.median(density)),densityP99=float(np.percentile(density,99)),minSeparation=float(separation.min()),medianSeparation=float(np.median(separation)),maxSeparation=float(separation.max()),seconds=time.monotonic()-start)
                db.execute('INSERT INTO surfaces VALUES(?,?,?,?)',(region,count,json.dumps(info),packed));db.commit();report.append(info);print(json.dumps(info),flush=True)
        (args.output/'report.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
    source.db.close()

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--source',type=Path,default=Path('build/terrain-lod.sqlite3'));p.add_argument('--regions',type=Path,default=Path('build/lpf-curve-smooth-prototype/comparison.sqlite3'));p.add_argument('--uniform',type=Path,default=Path('build/adaptive-free-surface-v7/adaptive.sqlite3'));p.add_argument('--output',type=Path,default=Path('build/adaptive-free-surface-v8'));p.add_argument('--region');p.add_argument('--budgets',type=int,nargs='+',default=BUDGETS);p.add_argument('--iterations',type=int,default=3);generate(p.parse_args())
