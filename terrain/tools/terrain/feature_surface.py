"""自由配置の異方性Gaussian曲面で地形を近似する地域試作。"""
import argparse
import base64
import functools
import gzip
import json
import math
import sqlite3
import time
import urllib.request
import zlib
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlparse,parse_qs

import numpy as np
from scipy.linalg import solve
from scipy.ndimage import gaussian_filter,maximum_filter
from scipy.optimize import minimize
from lpf_curve_compare import Source

SPAN=32000.
BUDGETS=[8,32,128,512]


def gaussian(x,y,p,derivative=False):
    cx,cy,rx,ry,theta=p[:5]
    cs,sn=np.cos(theta),np.sin(theta)
    dx,dy=x-cx,y-cy
    u=(dx*cs+dy*sn)/rx;v=(-dx*sn+dy*cs)/ry
    value=np.exp(-.5*(u*u+v*v))
    if not derivative:return value
    return value,-value*(u*cs/rx-v*sn/ry),-value*(u*sn/rx+v*cs/ry)


def evaluate(base,features,x,y,derivative=False):
    x,y=np.broadcast_arrays(np.asarray(x,dtype=float),np.asarray(y,dtype=float))
    h=base[0]+base[1]*x/SPAN+base[2]*y/SPAN
    gx=np.full(x.shape,base[1]/SPAN);gy=np.full(x.shape,base[2]/SPAN)
    for p in features:
        if derivative:
            v,dx,dy=gaussian(x,y,p,True);h+=p[5]*v;gx+=p[5]*dx;gy+=p[5]*dy
        else:h+=p[5]*gaussian(x,y,p)
    return (h,gx,gy) if derivative else h


def refine_feature(x,y,residual,weights,initial):
    """中心・幅・向きを連続値で調整する。高さは射影から求める。"""
    ids=np.arange(0,len(x),max(1,len(x)//4096))
    sx,sy,r,w=x[ids],y[ids],residual[ids],weights[ids]
    start=np.array([initial[0]/SPAN,initial[1]/SPAN,math.log(initial[2]/SPAN),math.log(initial[3]/SPAN),initial[4]])
    def objective(p):
        cx,cy=p[:2]*SPAN;rx,ry=np.exp(p[2:4])*SPAN;theta=p[4]
        cs,sn=np.cos(theta),np.sin(theta);dx,dy=sx-cx,sy-cy
        u=(dx*cs+dy*sn)/rx;v=(-dx*sn+dy*cs)/ry
        g=np.exp(-.5*(u*u+v*v));corr=np.sum(w*r*g);den=np.sum(w*g*g)+1e-8
        derivatives=np.column_stack([g*(u*cs/rx-v*sn/ry)*SPAN,g*(u*sn/rx+v*cs/ry)*SPAN,g*u*u,g*v*v,g*u*v*(rx/ry-ry/rx)])
        dc=derivatives.T@(w*r);dd=2*derivatives.T@(w*g)
        scale=max(1.,np.sum(w*r*r))
        return -corr*corr/den/scale,(-2*corr/den*dc+corr*corr/(den*den)*dd)/scale
    result=minimize(objective,start,jac=True,method='L-BFGS-B',bounds=[(-.5,.5),(-.5,.5),(math.log(250/SPAN),0),(math.log(250/SPAN),0),(-math.pi,math.pi)],options=dict(maxiter=12,ftol=1e-7))
    p=result.x
    candidate=np.array([p[0]*SPAN,p[1]*SPAN,np.exp(p[2])*SPAN,np.exp(p[3])*SPAN,p[4]])
    # 間引いた最適化点だけで良くなる候補は採用しない。
    def score(q):
        g=gaussian(x,y,q);return float(np.sum(weights*residual*g)**2/(np.sum(weights*g*g)+1e-8))
    return candidate if score(candidate)>=score(initial) else np.array(initial)


def fit(x,y,height,budgets=BUDGETS,callback=None):
    shape=height.shape
    x,y,h=x.ravel(),y.ravel(),height.ravel()
    smooth=gaussian_filter(height,1.5)
    gy,gx=np.gradient(smooth,SPAN/(shape[0]-1))
    gyy,gyx=np.gradient(gy,SPAN/(shape[0]-1));gxy,gxx=np.gradient(gx,SPAN/(shape[0]-1))
    detail=np.abs(height-gaussian_filter(height,5))
    weights=1+.75*np.clip(detail.ravel()/max(1.,np.percentile(detail,90)),0,1)
    # 卓越した山頂の周囲も評価する。1点の高さだけを合わせる制約にはしない。
    prominence=height-gaussian_filter(height,4)
    peaks=(height==maximum_filter(height,size=9))&(prominence>max(30.,.08*np.ptp(height)))
    weights+=12*maximum_filter(peaks.astype(float),size=3).ravel()
    design=np.empty((len(h),3+max(budgets)),float)
    design[:,:3]=np.column_stack([np.ones(len(h)),x/SPAN,y/SPAN])
    base=np.linalg.lstsq(design[:,:3]*np.sqrt(weights[:,None]),h*np.sqrt(weights),rcond=None)[0]
    residual=h-design[:,:3]@base
    features=[];models=[]
    for count in range(1,max(budgets)+1):
        index=int(np.argmax(np.abs(residual)*np.sqrt(weights)))
        hessian=np.array([[gxx.ravel()[index],gxy.ravel()[index]],[gyx.ravel()[index],gyy.ravel()[index]]])
        _,vectors=np.linalg.eigh(hessian)
        theta=math.atan2(vectors[1,0],vectors[0,0])
        candidates=[]
        for radius in [500,1000,2000,4000,8000,16000]:
            for aspect in [1,2,4]:
                for angle in ([theta] if aspect==1 else [theta,theta+math.pi/2]):
                    candidates.append([x[index],y[index],radius,min(32000,radius*aspect),((angle+math.pi)%(2*math.pi))-math.pi])
        scores=[]
        for p in candidates:
            g=gaussian(x,y,p);scores.append(float(np.sum(weights*residual*g)**2/(np.sum(weights*g*g)+1e-8)))
        p=refine_feature(x,y,residual,weights,np.array(candidates[int(np.argmax(scores))]))
        g=gaussian(x,y,p);amplitude=np.sum(weights*residual*g)/(np.sum(weights*g*g)+1e-8)
        features.append(np.r_[p,amplitude]);design[:,count+2]=g;residual-=amplitude*g
        if count in budgets or count%32==0:
            a=design[:,:count+3];gram=a.T@(weights[:,None]*a)
            gram.flat[::len(gram)+1]+=np.r_[np.full(3,1e-10),np.full(count,.001)]
            coefficients=solve(gram,a.T@(weights*h),assume_a='pos')
            base=coefficients[:3]
            for feature,value in zip(features,coefficients[3:]):feature[5]=value
            residual=h-a@coefficients
        if count in budgets:
            models.append((count,base.astype('<f4').copy(),np.array(features,dtype='<f4').copy()))
        if callback and (count in budgets or count%16==0):callback(count,float(np.sqrt(np.mean(residual**2))))
    return models


def generate(args):
    args.output.mkdir(parents=True,exist_ok=True)
    target=args.output/'features.sqlite3'
    if target.exists():raise RuntimeError('新しい出力先を指定してください')
    source=Source(args.source);db=sqlite3.connect(target)
    db.executescript('CREATE TABLE regions(name TEXT PRIMARY KEY,info TEXT);CREATE TABLE surfaces(region TEXT,budget INTEGER,info TEXT,base BLOB,features BLOB,PRIMARY KEY(region,budget)) WITHOUT ROWID;')
    meta=sqlite3.connect(args.base.resolve().as_uri()+'?mode=ro',uri=True)
    report=[]
    for name,raw in meta.execute('SELECT name,info FROM regions ORDER BY name'):
        center=np.array(json.loads(raw)['center']);db.execute('INSERT INTO regions VALUES(?,?)',(name,raw));db.commit()
        axis=np.linspace(-SPAN/2,SPAN/2,129);x,y=np.meshgrid(axis,axis)
        height=source.sample(x+center[0],y+center[1])
        start=time.monotonic()
        def progress(count,error):
            v=dict(region=name,curves=count,trainingRmse=error,seconds=time.monotonic()-start)
            print(json.dumps(v),flush=True);(args.output/'progress.json').write_text(json.dumps(v),encoding='utf-8')
        models=fit(x,y,height,callback=progress)
        axis=np.linspace(-SPAN/2,SPAN/2,513);ex,ey=np.meshgrid(axis,axis)
        reference=source.sample(ex+center[0],ey+center[1])
        rng=np.random.default_rng(734);rx=rng.uniform(-SPAN/2,SPAN/2,8192);ry=rng.uniform(-SPAN/2,SPAN/2,8192);rh=source.sample(rx+center[0],ry+center[1])
        profileX=np.linspace(-16000,16000,513)
        for count,base,features in models:
            value=evaluate(base,features,ex,ey);error=value-reference
            random_error=evaluate(base,features,rx,ry)-rh
            packed=zlib.compress(features.tobytes(),6)
            info=dict(region=name,budget=count,curves=len(features),rawBytes=base.nbytes+features.nbytes,storedBytes=base.nbytes+len(packed),rmse=float(np.sqrt(np.mean(error**2))),maxError=float(np.max(abs(error))),randomMaxError=float(np.max(abs(random_error))),evaluationGridMeters=62.5,trainingGridMeters=250.,profile=evaluate(base,features,profileX,np.zeros(513)).tolist(),centerHeight=float(evaluate(base,features,0,0)),centerReference=float(source.sample(center[0],center[1])))
            db.execute('INSERT INTO surfaces VALUES(?,?,?,?,?)',(name,count,json.dumps(info),base.tobytes(),packed));db.commit();report.append(info)
        print(name,'complete',flush=True)
    db.close();(args.output/'report.json').write_text(json.dumps(dict(levels=report,sqliteBytes=target.stat().st_size),indent=2),encoding='utf-8');(args.output/'progress.json').write_text('{"complete":true}',encoding='utf-8')


def serve(args):
    @functools.lru_cache(maxsize=16)
    def packet(region,budget):
        with sqlite3.connect((args.output/'features.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as db:
            row=db.execute('SELECT info,base,features FROM surfaces WHERE region=? AND budget=?',(region,budget)).fetchone()
        if row is None:raise ValueError('未生成の曲面です')
        info=json.loads(row[0]);features=np.frombuffer(zlib.decompress(row[2]),'<f4').reshape(-1,6)
        return gzip.compress(json.dumps(dict(info=info,span=SPAN,base=np.frombuffer(row[1],'<f4').tolist(),features=features.tolist())).encode(),1)
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            try:
                url=urlparse(self.path);q=parse_qs(url.query);encoding=None
                if url.path=='/api/features':body=packet(q.get('region',['fuji'])[0],int(q.get('budget',['128'])[0]));mime='application/json';encoding='gzip'
                elif url.path.startswith('/api/'):
                    port=61467 if url.path=='/api/spline' else 61465
                    with urllib.request.urlopen(f'http://127.0.0.1:{port}'+self.path,timeout=120) as response:body=response.read();mime=response.headers['Content-Type'];encoding=response.headers.get('Content-Encoding')
                elif url.path=='/':
                    with urllib.request.urlopen('http://127.0.0.1:61467/') as response:html=response.read().decode()
                    html=html.replace('階層Bスプライン／三角形を保存しない曲面','山の形を少数の自由配置曲面で近似')
                    html=html.replace('<option value="bspline" selected>','<option value="features" selected>自由配置の山体・尾根曲面</option><option value="bspline">')
                    html=html.replace('<button id="reset">','<label>曲面数 <select id="featureCount">'+''.join(f'<option value="{v}"'+(' selected' if v==128 else '')+f'>{v}</option>' for v in BUDGETS)+'</select></label><label><input type="checkbox" id="featureCenters">曲面の中心</label><button id="reset">')
                    html=html.replace('<script src="/lpf_curve_compare.js">','<script src="/feature_surface.js"></script><script src="/lpf_curve_compare.js">')
                    a=html.index('<div id="note">');b=html.index('</div>',a)
                    html=html[:a]+'<div id="note">自由な位置・向き・幅の楕円Gaussian曲面を重ね、山体・尾根・谷を近似します。中心は格子に固定しません。保存は曲面パラメーターのみ。読み込み・分割変更時にGPUで描画メッシュを生成し、ドラッグ中は生成済みメッシュを使います。左は元メッシュ。Bスプラインとも切り替え可能。曲面分割は描画精度で、保存容量を変えません。LOD・地域境界の接合は未実装。</div>'+html[b+6:]
                    body=html.encode();mime='text/html; charset=utf-8'
                elif url.path in ['/feature_surface.js','/bspline_surface.js','/lpf_curve_compare.js','/lpf_curve_raycast.js']:
                    body=Path(__file__).with_name(url.path[1:]).read_bytes();mime='text/javascript; charset=utf-8'
                else:self.send_error(404);return
                self.send_response(200);self.send_header('Content-Type',mime);self.send_header('Content-Length',str(len(body)));self.send_header('Cache-Control','no-cache')
                if encoding:self.send_header('Content-Encoding',encoding)
                self.end_headers();self.wfile.write(body)
            except Exception as e:self.send_error(400,str(e))
        def log_message(self,*args):pass
    print(f'http://127.0.0.1:{args.port}/',flush=True);ThreadingHTTPServer(('127.0.0.1',args.port),Handler).serve_forever()


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('action',choices=['generate','serve']);p.add_argument('--output',type=Path,default=Path('build/feature-surface-prototype-v2'));p.add_argument('--source',type=Path,default=Path('build/terrain-lod.sqlite3'));p.add_argument('--base',type=Path,default=Path('build/lpf-curve-smooth-prototype/comparison.sqlite3'));p.add_argument('--port',type=int,default=61468)
    a=p.parse_args();dict(generate=generate,serve=serve)[a.action](a)
