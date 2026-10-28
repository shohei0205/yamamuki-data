"""自由配置の C2 局所基底を、誤差の大きな位置へ追加する比較。"""
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
import numpy as np
from scipy.spatial import cKDTree
from scipy.sparse import coo_matrix,csr_matrix,hstack,vstack,diags
from scipy.sparse.linalg import lsmr

SPAN=32000.
BUDGETS=[512,2048,8192,32768]


def basis(xy,centers,radii):
    tree=cKDTree(xy);rows=[];cols=[];values=[]
    for k,(p,radius) in enumerate(zip(centers,radii)):
        ids=np.array(tree.query_ball_point(p,float(radius)),int)
        r=np.linalg.norm(xy[ids]-p,axis=1)/radius;s=np.maximum(0.,1-r)
        rows.extend(ids);cols.extend([k]*len(ids));values.extend(s**4*(4*r+1))
    return coo_matrix((values,(rows,cols)),shape=(len(xy),len(centers))).tocsr()


def evaluate(base,features,xy,gradient=False):
    xy=np.asarray(xy,float).reshape(-1,2);result=base[0]+xy@np.asarray(base[1:])/SPAN
    derivatives=np.tile(np.asarray(base[1:])/SPAN,(len(xy),1));tree=cKDTree(xy)
    for cx,cy,radius,weight in features:
        ids=np.array(tree.query_ball_point([cx,cy],float(radius)),int)
        delta=xy[ids]-[cx,cy];r=np.linalg.norm(delta,axis=1)/radius;s=np.maximum(0.,1-r)
        result[ids]+=weight*s**4*(4*r+1)
        if gradient:derivatives[ids]+=(-20*weight*s**3/radius**2)[:,None]*delta
    return (result,derivatives) if gradient else result


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
    v=np.asarray(model['vertices'],float);xy=v[:,:2];height=v[:,2]
    # 量子化後に同じXYへ重なった入力点は一つにする。
    xy,ids=np.unique(xy,axis=0,return_index=True);height=height[ids]
    protected=np.unique(np.asarray(model['vertices'])[model['corners'],:2],axis=0)
    selected=[int(cKDTree(xy).query(p)[1]) for p in protected];selected=list(dict.fromkeys(selected))
    plane=np.column_stack((np.ones(len(xy)),xy/SPAN));base=np.linalg.lstsq(plane,height,rcond=None)[0]
    residual=height-plane@base
    distance=cKDTree(xy[selected]).query(xy)[0]
    while len(selected)<min(256,len(xy)):
        index=int(np.argmax(distance));selected.append(index);distance=np.minimum(distance,np.linalg.norm(xy-xy[index],axis=1))
    fixed_radii=[]
    importance=np.ones(len(xy));importance[selected[:len(protected)]]=20.
    start=time.monotonic();models=[];previous_coefficients=None
    for requested in budgets:
        target=min(requested,len(xy))
        while True:
            centers=xy[selected];neighbors=cKDTree(centers).query(centers,k=min(8,len(centers)))[0]
            proposed=np.clip(neighbors[:,-1]*1.8,120.,8000.)
            fixed_radii.extend(proposed[len(fixed_radii):].tolist());radii=np.asarray(fixed_radii)
            a=hstack((csr_matrix(plane),basis(xy,centers,radii)),format='csr')
            weighted=a.multiply(importance[:,None]).tocsr();scaling=1/np.sqrt(np.asarray(weighted.power(2).sum(axis=0)).ravel()+.003**2)
            system=vstack((weighted,diags(np.full(a.shape[1],.003))),format='csr').multiply(scaling[None,:]).tocsr()
            initial=np.zeros(a.shape[1])
            if previous_coefficients is not None:initial[:len(previous_coefficients)]=previous_coefficients
            solved=lsmr(system,np.r_[height*importance,np.zeros(a.shape[1])],atol=1e-6,btol=1e-6,maxiter=1600,x0=initial/scaling)
            coefficients=solved[0]*scaling;previous_coefficients=coefficients.copy();base=coefficients[:3];residual=height-a@coefficients
            print('制御点',len(selected),'目標',target,'RMSE',float(np.sqrt(np.mean(residual**2))),flush=True)
            if len(selected)>=target or np.max(abs(residual))<=.5:break
            choose(xy,residual,selected,min(target-len(selected),max(256,len(selected))))
        base=base.astype('<f4');features=np.column_stack((centers,radii,coefficients[3:])).astype('<f4')
        # 他の保護点へ届かない小さなC2補正を加え、高さを保持する。
        peak_height=height[cKDTree(xy).query(protected)[1]];difference=peak_height-evaluate(base,features,protected)
        spacing=cKDTree(protected).query(protected,k=2)[0][:,1] if len(protected)>1 else np.full(1,1250.)
        correction=np.column_stack((protected,np.maximum(.5,np.minimum(500.,spacing*.4)),difference)).astype('<f4')
        features=np.vstack((features,correction))
        error=evaluate(base,features,xy)-height;peak_error=float(np.max(abs(evaluate(base,features,protected)-peak_height)))
        if peak_error>.01:raise ValueError('保護点の保持誤差が大きすぎます')
        packed=zlib.compress(base.tobytes()+features.tobytes(),6)
        summit=np.array(model['vertices'])[min(model['corners'],key=lambda k:np.linalg.norm(np.array(model['vertices'])[k,:2]))].tolist()
        info=dict(budget=requested,controls=len(selected),corrections=len(correction),basisCount=len(features),storedBytes=len(packed),rawBytes=12+features.nbytes,sourceControls=len(xy),rmse=float(np.sqrt(np.mean(error**2))),maxError=float(abs(error).max()),peakError=peak_error,uncorrectedPeakMaxError=float(abs(difference).max()),iterations=int(solved[2]),solverStop=int(solved[1]),seconds=time.monotonic()-start,summit=summit,source='saved-control-points',kernel='Wendland-C2')
        models.append((info,packed))
    return models


def bins(features,n=64):
    lists=[[] for _ in range(n*n)]
    for index,(x,y,radius,_) in enumerate(features):
        low=np.maximum(0,np.floor((np.array([x,y])-radius+SPAN/2)/SPAN*n).astype(int));high=np.minimum(n-1,np.floor((np.array([x,y])+radius+SPAN/2)/SPAN*n).astype(int))
        for yy in range(low[1],high[1]+1):
            for xx in range(low[0],high[0]+1):lists[yy*n+xx].append(index)
    count=np.array([len(v) for v in lists],'<u4');offset=np.r_[0,count.cumsum()[:-1]].astype('<u4')
    if count.max()>1024:raise ValueError('基底の重なりがGPUの上限を超えます')
    return np.column_stack((offset,count)).astype('<u4'),np.array([k for v in lists for k in v],'<u4')


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
    @functools.lru_cache(maxsize=9)
    def packet(region,budget):
        with sqlite3.connect((args.output/'adaptive.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as db:raw,blob=db.execute('SELECT info,model FROM surfaces WHERE region=? AND budget=?',(region,budget)).fetchone()
        data=np.frombuffer(zlib.decompress(blob),'<f4');features=data[3:].reshape(-1,4);lookup,indices=bins(features)
        enc=lambda a:base64.b64encode(a.tobytes()).decode()
        return gzip.compress(json.dumps(dict(info=json.loads(raw),span=SPAN,base=data[:3].tolist(),features=enc(features),lookup=enc(lookup),indices=enc(indices),binN=64,maxBin=int(lookup[:,1].max()))).encode(),1)
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            try:
                url=urlparse(self.path);q=parse_qs(url.query);encoding=None
                if url.path=='/api/adaptive':body=packet(q.get('region',['yari'])[0],int(q.get('budget',['32768'])[0]));mime='application/json';encoding='gzip'
                elif url.path=='/':
                    with urllib.request.urlopen('http://127.0.0.1:61472/') as response:html=response.read().decode()
                    html=html.replace('<option value="ridgeSpline" selected>','<option value="adaptive">自由配置の局所曲面（試作）</option><option value="ridgeSpline" selected>')
                    html=html.replace('<button id="ridgeFocus">','<label>自由配置の制御点 <select id="adaptiveBudget"><option value="512">512点</option><option value="2048">2,048点</option><option value="8192">8,192点</option><option value="32768" selected>最大32,768点</option></select></label><button id="ridgeFocus">')
                    html=html.replace('Bスプライン制御点','曲面の制御点').replace('<script src="/lpf_curve_compare.js">','<script src="/adaptive_surface.js"></script><script src="/lpf_curve_compare.js">')
                    a=html.index('<div id="note">');b=html.index('</div>',a)
                    html=html[:a]+'<div id="note">現在の標準は「制御点から一続きのBスプライン」です。自由配置の局所曲面は、少数点でクレーター状のくぼみが強くなるため試作として残します。Bスプラインの高さと陰影は同じ曲面から評価します。自由配置版の水色は基底中心を曲面上に表示した点、格子版の水色は係数制御点です。</div>'+html[b+6:];body=html.encode();mime='text/html; charset=utf-8'
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
    p=argparse.ArgumentParser();p.add_argument('action',choices=['generate','serve']);p.add_argument('--input',type=Path,default=Path('build/ridge-surface-half-budget-v2/ridge.sqlite3'));p.add_argument('--output',type=Path,default=Path('build/adaptive-free-surface-v4'));p.add_argument('--port',type=int,default=61473);a=p.parse_args();dict(generate=generate,serve=serve)[a.action](a)
