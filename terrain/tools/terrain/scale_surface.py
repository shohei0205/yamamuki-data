"""一様な解像度の自由配置局所曲面。三角形区画と点の厳密補間を使わない。"""
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

SPAN=32000.
BUDGETS=[512,2048,8192,32768]


def terms(q):
    u,v=np.asarray(q).T
    return np.column_stack((u*0+1,u,v,u*u,u*v,v*v))


def centers_for_budget(count,reference,origin,step,spacing):
    """面積を基準に配置し、勾配による密度差は弱く制限する。"""
    initial=qmc.Sobol(2,scramble=True,seed=2841).random_base2(int(np.ceil(np.log2(count))))[:count]
    centers=(initial-.5)*SPAN
    candidates=(qmc.Sobol(2,scramble=True,seed=7183).random_base2(max(16,int(np.ceil(np.log2(count*4)))))-.5)*SPAN
    gy,gx=np.gradient(reference,step)
    ij=((candidates-origin)/step).T[[1,0]]
    slope=np.hypot(map_coordinates(gx,ij,order=1,prefilter=False),map_coordinates(gy,ij,order=1,prefilter=False))
    weights=1+.5*np.tanh(slope/.4)
    for _ in range(3):
        labels=cKDTree(centers).query(candidates)[1];mass=np.bincount(labels,weights,minlength=count)
        mean=np.column_stack([np.bincount(labels,weights*candidates[:,k],minlength=count) for k in range(2)])/np.maximum(mass[:,None],1e-20)
        move=mean-centers;move[mass==0]=0
        length=np.linalg.norm(move,axis=1);move*=np.minimum(1.,spacing*.2/np.maximum(length,1e-20))[:,None]
        centers=np.clip(centers+move,-SPAN/2,SPAN/2)
    return centers


def fit_patches(reference,origin,step,centers,radius):
    yy,xx=np.indices(reference.shape);positions=np.column_stack((xx.ravel()*step+origin,yy.ravel()*step+origin));tree=cKDTree(positions)
    neighbors=tree.query_ball_point(centers,radius)
    result=np.empty((len(centers),9),'<f4')
    for k,ids in enumerate(neighbors):
        q=(positions[ids]-centers[k])/radius;a=terms(q);w=np.exp(-np.sum(q*q,axis=1)/.25)
        normal=a.T@(w[:,None]*a);rhs=a.T@(w*reference.ravel()[ids])
        normal+=np.diag([1e-12,1e-10,1e-10,1e-8,1e-8,1e-8])
        coefficients=np.linalg.solve(normal,rhs)
        result[k]=[*centers[k],radius,*coefficients]
    return result


def evaluate(patches,xy,gradient=False):
    xy=np.asarray(xy,float).reshape(-1,2);queries=cKDTree(xy)
    weight=np.zeros(len(xy));height=np.zeros(len(xy));dw_sum=np.zeros((len(xy),2));dn=np.zeros((len(xy),2))
    for cx,cy,radius,*c in patches:
        ids=np.asarray(queries.query_ball_point([cx,cy],float(radius)),int);q=(xy[ids]-[cx,cy])/radius;r=np.linalg.norm(q,axis=1);s=np.maximum(0.,1-r)
        confidence=c[6] if len(c)>6 else 1.
        w=confidence*s**4*(4*r+1);p=terms(q)@np.asarray(c[:6]);weight[ids]+=w;height[ids]+=w*p
        if gradient:
            dw=(-20*confidence*s**3/radius)[:,None]*q
            dp=np.column_stack((c[1]+2*c[3]*q[:,0]+c[4]*q[:,1],c[2]+c[4]*q[:,0]+2*c[5]*q[:,1]))/radius
            dw_sum[ids]+=dw;dn[ids]+=dw*p[:,None]+w[:,None]*dp
    if np.any(weight<1e-12):raise ValueError('自由配置曲面が領域を覆っていません')
    h=height/weight
    return (h,(dn-h[:,None]*dw_sum)/weight[:,None]) if gradient else h


def bins(patches,n=128):
    lists=[[] for _ in range(n*n)]
    for k,p in enumerate(patches):
        low=np.clip(np.floor((p[:2]-p[2]+SPAN/2)/SPAN*n).astype(int),0,n-1);high=np.clip(np.floor((p[:2]+p[2]+SPAN/2)/SPAN*n).astype(int),0,n-1)
        for y in range(low[1],high[1]+1):
            for x in range(low[0],high[0]+1):lists[y*n+x].append(k)
    counts=np.asarray([len(v) for v in lists],'<u4');offsets=np.r_[0,counts.cumsum()[:-1]].astype('<u4')
    if counts.max()>1024:raise ValueError('GPU検索の上限を超えます')
    return np.column_stack((offsets,counts)).astype('<u4'),np.asarray([k for ids in lists for k in ids],'<u4')


def generate(args):
    args.output.mkdir(parents=True,exist_ok=True);path=args.output/'adaptive.sqlite3'
    if path.exists():raise ValueError('旧データを保持するため新しい出力先を指定してください')
    source=Source(args.source);step=SPAN/1024;halo=160;origin=-SPAN/2-halo*step
    axis=np.arange(1025+2*halo)*step+origin;xx,yy=np.meshgrid(axis,axis)
    with sqlite3.connect(args.regions.resolve().as_uri()+'?mode=ro',uri=True) as meta,sqlite3.connect(path) as db:
        db.execute('CREATE TABLE surfaces(region TEXT,budget INTEGER,info TEXT,model BLOB,PRIMARY KEY(region,budget)) WITHOUT ROWID');report=[]
        for region,raw in meta.execute('SELECT name,info FROM regions ORDER BY name'):
            if args.region and region!=args.region:continue
            center=np.asarray(json.loads(raw)['center']);print(region,'元標高を読む',flush=True)
            reference=source.sample(xx+center[0],yy+center[1]);start=time.monotonic()
            eval_axis=np.linspace(-SPAN/2,SPAN/2,257);ex,ey=np.meshgrid(eval_axis,eval_axis);query=np.column_stack((ex.ravel(),ey.ravel()))
            original=map_coordinates(reference,((query-origin)/step).T[[1,0]],order=1,prefilter=False)
            for count in args.budgets:
                spacing=SPAN/np.sqrt(count);sigma=spacing*.3;radius=spacing*1.6
                print(region,count,'全域を同じ尺度でならす',sigma,flush=True)
                target=gaussian_filter(reference,sigma/step,mode='nearest',truncate=4.)
                centers=centers_for_budget(count,target,origin,step,spacing)
                print(region,count,'局所曲面を近似',flush=True)
                patches=fit_patches(target[::2,::2],origin,step*2,centers,radius)
                h=evaluate(patches,query);smooth=map_coordinates(target,((query-origin)/step).T[[1,0]],order=1,prefilter=False)
                separation=cKDTree(centers).query(centers,k=2)[0][:,1];density=cKDTree(centers).query_ball_point(centers,spacing*2,return_length=True)
                packed=zlib.compress(patches.tobytes(),6);summit_height=float(evaluate(patches,[[0,0]])[0])
                info=dict(region=region,budget=count,controls=count,corrections=0,storedBytes=len(packed),rawBytes=patches.nbytes,kernel='Scale-C2-local-quadratic',source='DEM-z14-generation-only',spacingMeters=float(spacing),sigmaMeters=float(sigma),radiusMeters=float(radius),rmse=float(np.sqrt(np.mean((h-original)**2))),maxError=float(np.max(abs(h-original))),filteredRmse=float(np.sqrt(np.mean((h-smooth)**2))),filteredMaxError=float(np.max(abs(h-smooth))),peakError=float(abs(summit_height-source.sample(center[0],center[1]).item())),summit=[0.,0.,summit_height],minSeparation=float(separation.min()),medianSeparation=float(np.median(separation)),maxSeparation=float(separation.max()),densityP50=float(np.median(density)),densityP99=float(np.percentile(density,99)),evaluationGridMeters=125.,seconds=time.monotonic()-start)
                db.execute('INSERT INTO surfaces VALUES(?,?,?,?)',(region,count,json.dumps(info),packed));db.commit();report.append(info);print(json.dumps(info),flush=True)
        (args.output/'report.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
    source.db.close()

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--source',type=Path,default=Path('build/terrain-lod.sqlite3'));p.add_argument('--regions',type=Path,default=Path('build/lpf-curve-smooth-prototype/comparison.sqlite3'));p.add_argument('--output',type=Path,default=Path('build/adaptive-free-surface-v7'));p.add_argument('--budgets',type=int,nargs='+',default=BUDGETS);p.add_argument('--region');generate(p.parse_args())
