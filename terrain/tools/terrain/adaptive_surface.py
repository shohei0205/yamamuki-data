"""自由配置点から C1 連続の三次曲面を復元し、GPU で描画する。"""
import argparse
import base64
import functools
import gzip
import json
import sqlite3
import time
import urllib.request
import zlib
from pathlib import Path
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from urllib.parse import urlparse,parse_qs
import os
os.environ.setdefault("OPENBLAS_NUM_THREADS","4")
os.environ.setdefault("MKL_NUM_THREADS","4")
import numpy as np
from scipy.spatial import cKDTree
from scipy.interpolate import CloughTocher2DInterpolator
from scipy.spatial.distance import cdist
from scipy.linalg import solve

SPAN=32000.
BUDGETS=[512,2048,8192,32768]


def choose(xy,error,selected,count):
    """既存点と重複せず、残差が大きい位置を優先する。"""
    distance=cKDTree(xy[selected]).query(xy)[0] if selected else np.full(len(xy),SPAN)
    mask=np.zeros(len(xy),bool);mask[selected]=True;chosen=[]
    score=np.abs(error).copy()
    for _ in range(count):
        priority=score*np.minimum(1.,distance/100.);priority[mask]=-1
        index=int(np.argmax(priority))
        if priority[index]<0:break
        selected.append(index);chosen.append(index);mask[index]=True
        distance=np.minimum(distance,np.linalg.norm(xy-xy[index],axis=1))
    return chosen


def fit_models(model,budgets=BUDGETS):
    v=np.asarray(model['vertices'],float);xy,ids=np.unique(v[:,:2],axis=0,return_index=True);height=v[ids,2]
    tree=cKDTree(xy);protected=np.unique(v[model['corners'],:2],axis=0)
    selected=list(dict.fromkeys(tree.query(protected)[1].tolist()))
    distance=cKDTree(xy[selected]).query(xy)[0]
    while len(selected)<min(128,len(xy)):
        i=int(np.argmax(distance));selected.append(i);distance=np.minimum(distance,np.linalg.norm(xy-xy[i],axis=1))
    models=[];start=time.monotonic()
    for requested in budgets:
        target=min(requested,len(xy))
        while True:
            curve=CloughTocher2DInterpolator(xy[selected],height[selected],tol=1e-8,maxiter=1000)
            error=height-curve(xy)
            if not np.all(np.isfinite(error)):raise ValueError('曲面が領域を覆っていません')
            if len(selected)>=target:break
            choose(xy,error,selected,min(target-len(selected),128))
        points=np.column_stack((xy[selected],height[selected])).astype('<f4');packed=zlib.compress(points.tobytes(),6)
        summit=v[min(model['corners'],key=lambda k:np.linalg.norm(v[k,:2]))].tolist()
        info=dict(budget=requested,controls=len(selected),corrections=0,basisCount=len(selected),storedBytes=len(packed),rawBytes=points.nbytes,sourceControls=len(xy),rmse=float(np.sqrt(np.mean(error**2))),maxError=float(abs(error).max()),peakError=float(np.max(abs(curve(protected)-height[tree.query(protected)[1]]))),seconds=time.monotonic()-start,summit=summit,source='saved-control-points',kernel='Clough-Tocher-C1-cubic')
        print(json.dumps(info),flush=True);models.append((info,packed))
    return models


def cubic_terms(uv):
    u,v=np.asarray(uv).T
    return np.column_stack((u*0+1,u,v,u*u,u*v,v*v,u**3,u*u*v,u*v*v,v**3))


def cubic_packet(points,n=128):
    curve=CloughTocher2DInterpolator(points[:,:2],points[:,2],tol=1e-8,maxiter=1000)
    faces=curve.tri.simplices;v=points[faces,:2].astype(float);center=v.mean(axis=1)
    # Clough–Tocher の三次曲面を重心で3区画へ分け、同じ多項式をGPUへ渡す。
    sub=np.concatenate([np.stack((v[:,k],v[:,(k+1)%3],center),axis=1) for k in range(3)])
    uv=np.array([(i/3,j/3) for i in range(4) for j in range(4-i)])
    inv=np.linalg.inv(cubic_terms(uv));at=sub[:,2,None,:]+uv[None,:,0,None]*(sub[:,0,None,:]-sub[:,2,None,:])+uv[None,:,1,None]*(sub[:,1,None,:]-sub[:,2,None,:])
    values=curve(at.reshape(-1,2)).reshape(-1,10)
    # 凸包の辺上が丸め誤差で領域外になる場合のみ、同じ区画の内側へ寄せる。
    bad=~np.isfinite(values)
    if np.any(bad):
        inside=at+(sub.mean(axis=1)[:,None,:]-at)*1e-9
        values[bad]=curve(inside[bad])
    origin=values[:,0];coeff=(values-origin[:,None])@inv.T;coeff[:,0]+=origin
    triangles=np.column_stack((sub.reshape(-1,6),coeff)).astype('<f4')
    if not np.all(np.isfinite(triangles)):raise ValueError('三次曲面の復元に非有限値があります')
    lists=[[] for _ in range(n*n)]
    for k,tri in enumerate(sub):
        lo=np.clip(np.floor((tri.min(axis=0)+SPAN/2)/SPAN*n).astype(int),0,n-1);hi=np.clip(np.floor((tri.max(axis=0)+SPAN/2)/SPAN*n).astype(int),0,n-1)
        for y in range(lo[1],hi[1]+1):
            for x in range(lo[0],hi[0]+1):lists[y*n+x].append(k)
    counts=np.asarray([len(v) for v in lists],'<u4');offset=np.r_[0,counts.cumsum()[:-1]].astype('<u4')
    if counts.max()>1024:raise ValueError('曲面の検索候補がGPU上限を超えます')
    lookup=np.column_stack((offset,counts)).astype('<u4');indices=np.asarray([k for v in lists for k in v],'<u4')
    return triangles,lookup,indices


def thin_plate(points,smoothing=1e-7):
    """全点を使う薄板スプライン。近傍の切替や三角形区画を使わない。"""
    if len(points)>2048:raise ValueError('薄板スプラインの比較は2048点以下です')
    xy=points[:,:2].astype(float)/SPAN;r2=cdist(xy,xy,'sqeuclidean')
    kernel=.5*r2*np.log(np.maximum(r2,1e-30));polynomial=np.column_stack((np.ones(len(xy)),xy))
    system=np.block([[kernel+np.eye(len(xy))*smoothing,polynomial],[polynomial.T,np.zeros((3,3))]])
    coefficients=solve(system,np.r_[points[:,2],0.,0.,0.],assume_a='sym')
    features=np.column_stack((points[:,:2],np.zeros(len(points)),coefficients[:-3])).astype('<f4')
    base=coefficients[-3:].astype('<f4')
    return base,features


def thin_plate_values(base,features,xy):
    xy=np.asarray(xy,float);values=np.empty(len(xy))
    for start in range(0,len(xy),2048):
        q=xy[start:start+2048]/SPAN;r2=cdist(q,features[:,:2].astype(float)/SPAN,'sqeuclidean')
        values[start:start+len(q)]=base[0]+q@base[1:]+(.5*r2*np.log(np.maximum(r2,1e-30)))@features[:,3]
    return values


def thin_plate_grid(base,features,n=1024):
    """大きな係数の打消しをCPU倍精度で計算し、描画用に一時保持する。"""
    axis=np.linspace(-SPAN/2,SPAN/2,n+1);x,y=np.meshgrid(axis,axis);xy=np.column_stack((x.ravel(),y.ravel()))/SPAN
    centers=features[:,:2].astype(float)/SPAN;weights=features[:,3].astype(float);grid=np.zeros((len(xy),4),'<f4')
    for start in range(0,len(xy),1024):
        q=xy[start:start+1024];dx=q[:,0,None]-centers[None,:,0];dy=q[:,1,None]-centers[None,:,1]
        r2=np.maximum(dx*dx+dy*dy,1e-30);l=np.log(r2)
        grid[start:start+len(q),0]=base[0]+q@base[1:]+(.5*r2*l)@weights
        grid[start:start+len(q),1]=base[1]/SPAN+((l+1)*dx)@weights/SPAN
        grid[start:start+len(q),2]=base[2]/SPAN+((l+1)*dy)@weights/SPAN
    return grid


def generate(args):
    args.output.mkdir(parents=True,exist_ok=True);path=args.output/'adaptive.sqlite3'
    if path.exists():raise ValueError('新しい出力先を指定してください')
    with sqlite3.connect(args.input.resolve().as_uri()+'?mode=ro',uri=True) as source,sqlite3.connect(path) as db:
        db.execute('CREATE TABLE surfaces(region TEXT,budget INTEGER,info TEXT,model BLOB,PRIMARY KEY(region,budget)) WITHOUT ROWID');report=[]
        for region,blob in source.execute('SELECT region,model FROM surfaces ORDER BY region'):
            print(region,'自由配置の近似',flush=True)
            for info,packed in fit_models(json.loads(zlib.decompress(blob))):
                info['region']=region;db.execute('INSERT INTO surfaces VALUES(?,?,?,?)',(region,info['budget'],json.dumps(info),packed));db.commit();report.append(info);print(json.dumps(info),flush=True)
        (args.output/'report.json').write_text(json.dumps(report,indent=2),encoding='utf-8')


def serve(args):
    @functools.lru_cache(maxsize=18)
    def packet(region,budget,low_surface,layout):
        if layout in ('scale','adaptive'):
            from scale_surface import bins as scale_bins
            output=args.adaptive_output if layout=='adaptive' else args.scale_output
            with sqlite3.connect((output/'adaptive.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as db:raw,blob=db.execute('SELECT info,model FROM surfaces WHERE region=? AND budget=?',(region,budget)).fetchone()
            width=10 if layout=='adaptive' else 9
            patches=np.frombuffer(zlib.decompress(blob),'<f4').reshape(-1,width);info=json.loads(raw);lookup,indices=scale_bins(patches)
            features=patches[:,[0,1,2,3]].copy();gpu=np.zeros((len(patches),12),'<f4');gpu[:,:width]=patches
            enc=lambda a:base64.b64encode(a.tobytes()).decode()
            return gzip.compress(json.dumps(dict(info=info,span=SPAN,base=[0,0,0],kernelMode=3 if layout=='adaptive' else 2,fieldN=0,fieldGrid=enc(np.zeros(4,'<f4')),features=enc(features),triangles=enc(gpu),lookup=enc(lookup),indices=enc(indices),binN=128,maxBin=int(lookup[:,1].max()))).encode(),1)
        with sqlite3.connect((args.output/'adaptive.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as db:raw,blob=db.execute('SELECT info,model FROM surfaces WHERE region=? AND budget=?',(region,budget)).fetchone()
        data=np.frombuffer(zlib.decompress(blob),'<f4').reshape(-1,3);info=json.loads(raw)
        mode=1 if len(data)<=2048 and low_surface=='thinPlate' else 0
        if mode:
            base,features=thin_plate(data);field_grid=thin_plate_grid(base,features);triangles=np.zeros((1,16),'<f4');lookup=np.zeros((128*128,2),'<u4');indices=np.zeros(1,'<u4')
            with sqlite3.connect(args.input.resolve().as_uri()+'?mode=ro',uri=True) as source:
                source_model=json.loads(zlib.decompress(source.execute('SELECT model FROM surfaces WHERE region=?',(region,)).fetchone()[0]))
            vertices=np.asarray(source_model['vertices'],float);error=thin_plate_values(base,features,vertices[:,:2])-vertices[:,2]
            protected=vertices[source_model['corners']]
            info.update(kernel='Thin-plate-global',rmse=float(np.sqrt(np.mean(error**2))),maxError=float(np.max(abs(error))),peakError=float(np.max(abs(thin_plate_values(base,features,protected[:,:2])-protected[:,2]))),regularization=1e-7)
        else:
            field_grid=np.zeros((1,4),'<f4');base=np.zeros(3,'<f4');features=np.column_stack((data[:,:2],np.zeros(len(data)),data[:,2])).astype('<f4');triangles,lookup,indices=cubic_packet(data)
        enc=lambda a:base64.b64encode(a.tobytes()).decode()
        return gzip.compress(json.dumps(dict(info=info,span=SPAN,base=base.tolist(),kernelMode=mode,fieldN=1024 if mode else 0,fieldGrid=enc(field_grid),features=enc(features),triangles=enc(triangles),lookup=enc(lookup),indices=enc(indices),binN=128,maxBin=int(lookup[:,1].max()))).encode(),1)

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            try:
                url=urlparse(self.path);q=parse_qs(url.query);encoding=None
                if url.path=='/api/adaptive':body=packet(q.get('region',['yari'])[0],int(q.get('budget',['32768'])[0]),q.get('lowSurface',['thinPlate'])[0],q.get('layout',['adaptive'])[0]);mime='application/json';encoding='gzip'
                elif url.path=='/':
                    with urllib.request.urlopen('http://127.0.0.1:61472/') as response:html=response.read().decode()
                    html=html.replace('<option value="ridgeSpline" selected>','<option value="adaptive" selected>自由配置の曲面</option><option value="ridgeSpline">')
                    html=html.replace('<button id="ridgeFocus">','<label>点の配分と曲面 <select id="adaptiveLayout"><option value="adaptive" selected>LPF共通・誤差で点を再配分（新）</option><option value="scale">ほぼ均等配置（比較元）</option><option value="legacy">誤差優先の点配置（従来）</option></select></label><label>少数点の曲面 <select id="adaptiveLowSurface"><option value="thinPlate" selected>薄板（区画なし・2048点以下）</option><option value="cubic">三次曲面（従来）</option></select></label><label>曲面のならし <select id="adaptiveFairing"><option value="0">なし（従来）</option><option value="40" selected>40 m</option><option value="80">80 m</option><option value="120">120 m</option><option value="160">160 m</option></select></label><label>自由配置の制御点 <select id="adaptiveBudget"><option value="512">512点</option><option value="2048">2,048点</option><option value="8192">8,192点</option><option value="32768" selected>最大32,768点</option></select></label><button id="ridgeFocus">')
                    html=html.replace('Bスプライン制御点','曲面の制御点').replace('<script src="/lpf_curve_compare.js">','<script src="/adaptive_surface.js"></script><script src="/lpf_curve_compare.js">')
                    a=html.index('<div id="note">');b=html.index('</div>',a)
                    html=html[:a]+'<div id="note">誤差配分版は共通LPFを維持し、平坦部の曲面を広げて、再現誤差が大きな尾根・谷へ点を回します。別の評価点で改善した候補を採用します。三角形で区切らず、自由配置した局所曲面をC2の重みで滑らかに合成します。点を減らすほど全域で同じ尺度の細部を落とし、山頂だけ厳密に通す補正は使いません。点の密度差も弱く制限します。格子は生成時の標高参照と描画用だけで、保存する中心は自由配置です。新方式の保存量には中心・半径・局所二次曲面の係数を含みます。水色は曲面上の中心位置です。従来方式も選べます。</div>'+html[b+6:];body=html.encode();mime='text/html; charset=utf-8'
                elif url.path=='/adaptive_surface.js':body=Path(__file__).with_suffix('.js').read_bytes();mime='text/javascript'
                else:
                    with urllib.request.urlopen('http://127.0.0.1:61472'+self.path,timeout=180) as response:body=response.read();mime=response.headers['Content-Type'];encoding=response.headers.get('Content-Encoding')
                self.send_response(200);self.send_header('Content-Type',mime);self.send_header('Content-Length',str(len(body)));self.send_header('Cache-Control','no-cache')
                if encoding:self.send_header('Content-Encoding',encoding)
                self.end_headers();self.wfile.write(body)
            except (BrokenPipeError,ConnectionAbortedError,ConnectionResetError):pass
            except Exception as error:self.send_error(400,'Request failed',explain=str(error))
        def log_message(self,*args):pass
    print(f'http://127.0.0.1:{args.port}/',flush=True);ThreadingHTTPServer(('127.0.0.1',args.port),Handler).serve_forever()


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('action',choices=['generate','serve']);p.add_argument('--input',type=Path,default=Path('build/ridge-surface-half-budget-v2/ridge.sqlite3'));p.add_argument('--output',type=Path,default=Path('build/adaptive-free-surface-v6'));p.add_argument('--adaptive-output',type=Path,default=Path('build/adaptive-free-surface-v8'));p.add_argument('--scale-output',type=Path,default=Path('build/adaptive-free-surface-v7'));p.add_argument('--port',type=int,default=61473);a=p.parse_args();dict(generate=generate,serve=serve)[a.action](a)
