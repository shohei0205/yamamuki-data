"""保存した骨格制御点を、一続きの3次Bスプラインへ近似する比較。"""
import argparse
import base64
import functools
import gzip
import json
import sqlite3
import time
import urllib.request
import zlib
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlparse,parse_qs

import numpy as np
from scipy.sparse import csr_matrix,coo_matrix,vstack,eye,diags,kron
from scipy.sparse.linalg import lsmr
from scipy.linalg import qr

SPAN=32000.


def design(xy,n):
    q=(np.asarray(xy)+SPAN/2)*n/SPAN+2;cell=np.floor(q).astype(int);t=q-cell
    def basis(t):return np.column_stack(((1-t)**3,3*t**3-6*t*t+4,-3*t**3+3*t*t+3*t+1,t**3))/6
    bx,by=basis(t[:,0]),basis(t[:,1]);cols=[];values=[]
    for j in range(4):
        for i in range(4):cols.append((cell[:,1]+j-1)*(n+5)+cell[:,0]+i-1);values.append(bx[:,i]*by[:,j])
    return csr_matrix((np.column_stack(values).ravel(),(np.repeat(np.arange(len(xy)),16),np.column_stack(cols).ravel())),shape=(len(xy),(n+5)**2))


def fit(model,n,regularization=.08):
    start=time.monotonic();vertices=np.array(model['vertices']);a=design(vertices[:,:2],n);m=a.shape[1]
    peak_ids=np.array(sorted(set(model['corners'])),int);p=a[peak_ids];used=set();pivots=[]
    for row in range(p.shape[0]):
        s=p.getrow(row);options=sorted(zip(s.indices,s.data),key=lambda item:-abs(item[1]))
        pivot=next((int(i) for i,w in options if i not in used and w>1e-8),None)
        if pivot is None:raise ValueError('この間隔では山頂の制約を独立に保持できません')
        pivots.append(pivot);used.add(pivot)
    pivots=np.array(pivots);free=np.setdiff1d(np.arange(m),pivots);c=p[:,pivots].toarray()
    if np.linalg.cond(c)>1e8:
        eligible=np.unique(p.indices);_,_,order=qr(p[:,eligible].toarray(),mode='economic',pivoting=True)
        pivots=eligible[order[:len(peak_ids)]];free=np.setdiff1d(np.arange(m),pivots);c=p[:,pivots].toarray()
    if np.linalg.cond(c)>1e8:raise ValueError('山頂制約の条件が悪いため、細かい間隔を選んでください')
    inverse=np.linalg.inv(c);inverse[abs(inverse)<1e-12]=0
    fixed=-csr_matrix(inverse)@p[:,free];coo=fixed.tocoo()
    transform=coo_matrix((np.r_[np.ones(len(free)),coo.data],(np.r_[free,pivots[coo.row]],np.r_[np.arange(len(free)),coo.col])),shape=(m,len(free))).tocsr()
    offset=np.zeros(m);offset[pivots]=inverse@vertices[peak_ids,2]
    size=n+5;second=diags([np.ones(size-2),-2*np.ones(size-2),np.ones(size-2)],[0,1,2],shape=(size-2,size),format='csr')
    first=diags([-np.ones(size-1),np.ones(size-1)],[0,1],shape=(size-1,size),format='csr')
    # 三角形内の形状は使わず、全域の曲率にだけ弱い制約を加える。
    penalty=vstack((kron(eye(size),second),kron(second,eye(size)),np.sqrt(2)*kron(first,first)),format='csr')*regularization
    system=vstack((a@transform,penalty@transform),format='csr');rhs=np.r_[vertices[:,2]-a@offset,-penalty@offset]
    result=lsmr(system,rhs,atol=2e-6,btol=2e-6,maxiter=4000)
    coefficients=(transform@result[0]+offset).reshape(size,size).astype('<f4');restored=a@coefficients.ravel();error=restored-vertices[:,2]
    peak_error=float(np.max(abs(error[peak_ids])))
    if peak_error>.01:raise ValueError(f'山頂の保持誤差が大きすぎます: {peak_error}')
    packed=zlib.compress(coefficients.tobytes(),6)
    info=dict(summit=vertices[min(model['corners'],key=lambda v:vertices[v,0]**2+vertices[v,1]**2)].tolist(),n=n,stepMeters=SPAN/n,cumulativeCoefficients=int(coefficients.size),cumulativeBytes=len(packed),denseGpuBytes=coefficients.nbytes,rmse=float(np.sqrt(np.mean(error**2))),maxError=float(abs(error).max()),peakError=peak_error,sourceControls=len(vertices),sourceBytes=model['info']['storedBytes'],fitSource='ridge-control-points',sourceQuality=model['info']['quality'],iterations=int(result[2]),solverStop=int(result[1]),regularization=regularization,seconds=time.monotonic()-start)
    return info,packed


def generate(args):
    args.output.mkdir(parents=True,exist_ok=True);target=args.output/'spline.sqlite3'
    if target.exists():raise ValueError('新しい出力先を指定してください')
    with sqlite3.connect(args.input.resolve().as_uri()+'?mode=ro',uri=True) as source,sqlite3.connect(target) as db:
        db.execute('CREATE TABLE surfaces(region TEXT,quality TEXT,n INTEGER,info TEXT,coefficients BLOB,PRIMARY KEY(region,quality,n)) WITHOUT ROWID')
        report=[]
        for region,raw,blob in source.execute('SELECT region,info,model FROM surfaces ORDER BY region'):
            model=json.loads(zlib.decompress(blob));model['info']=json.loads(raw)
            for n in [128,256,512]:
                print(region,n,'制御点からBスプラインを近似',flush=True);info,packed=fit(model,n);info['region']=region
                db.execute('INSERT INTO surfaces VALUES(?,?,?,?,?)',(region,model['info']['quality'],n,json.dumps(info),packed));db.commit();report.append(info);print(json.dumps(info),flush=True)
        (args.output/'report.json').write_text(json.dumps(report,indent=2),encoding='utf-8')


def serve(args):
    @functools.lru_cache(maxsize=9)
    def packet(region,n):
        with sqlite3.connect((args.output/'spline.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as db:row=db.execute('SELECT info,coefficients FROM surfaces WHERE region=? AND n=?',(region,n)).fetchone()
        if row is None:raise ValueError('未生成の曲面です')
        info=json.loads(row[0]);body=dict(info=info,size=n+5,halo=2,span=SPAN,summit=info['summit'],coefficients=base64.b64encode(zlib.decompress(row[1])).decode())
        return gzip.compress(json.dumps(body).encode(),1)
    @functools.lru_cache(maxsize=3)
    def input_packet(region):
        with sqlite3.connect(args.input.resolve().as_uri()+'?mode=ro',uri=True) as db:row=db.execute('SELECT model FROM surfaces WHERE region=?',(region,)).fetchone()
        if row is None:raise ValueError('未生成の地域です')
        vertices=np.asarray(json.loads(zlib.decompress(row[0]))['vertices'],dtype='<f4')
        return gzip.compress(json.dumps(dict(region=region,count=len(vertices),points=base64.b64encode(vertices.tobytes()).decode())).encode(),1)
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            try:
                url=urlparse(self.path);q=parse_qs(url.query);encoding=None
                if url.path=='/api/ridge-spline':body=packet(q.get('region',['yari'])[0],int(q.get('n',['256'])[0]));mime='application/json';encoding='gzip'
                elif url.path=='/api/ridge-spline-input':body=input_packet(q.get('region',['yari'])[0]);mime='application/json';encoding='gzip'
                elif url.path=='/':
                    with urllib.request.urlopen('http://127.0.0.1:61470/') as r:html=r.read().decode()
                    html=html.replace('<option value="ridge" selected>','<option value="ridgeSpline" selected>制御点から一続きのBスプライン</option><option value="ridge">')
                    html=html.replace('<button id="ridgeFocus">','<label>制御点Bスプラインの間隔 <select id="ridgeSplineQuality"><option value="128">250 m</option><option value="256" selected>125 m</option><option value="512">62.5 m</option></select></label><button id="ridgeFocus">')
                    html=html.replace('<button id="reset">','<label><input type="checkbox" id="splineControlPoints" checked>Bスプライン制御点</label><label><input type="checkbox" id="splineInputPoints">近似の入力点</label><label><input type="checkbox" id="splinePointXray" checked>地形越しに点を表示</label><span id="splinePointsLegend"></span><button id="reset">')
                    a=html.index('<div id="note">');b=html.index('</div>',a)
                    html=html[:a]+'<div id="note">保存した制御点のXYZだけから、一続きの3次Bスプライン曲面を近似。三角形・四角形の面、辺、法線を近似に使いません。山頂と外周角の高さは制約で保持。全域で曲率を弱く抑えます。高さと法線を同じBスプラインからGPUで評価するため、元の面境界で折れません。左は元TIN。元DEMとの誤差保証・全国LOD接合は未実装。</div>'+html[b+6:];body=html.encode();mime='text/html; charset=utf-8'
                elif url.path in ['/lpf_curve_compare.js','/bspline_surface.js','/ridge_surface.js','/feature_surface.js','/lpf_curve_raycast.js']:body=Path(__file__).with_name(url.path[1:]).read_bytes();mime='text/javascript'
                else:
                    with urllib.request.urlopen('http://127.0.0.1:61470'+self.path,timeout=180) as r:body=r.read();mime=r.headers['Content-Type'];encoding=r.headers.get('Content-Encoding')
                self.send_response(200);self.send_header('Content-Type',mime);self.send_header('Content-Length',str(len(body)));self.send_header('Cache-Control','no-cache')
                if encoding:self.send_header('Content-Encoding',encoding)
                self.end_headers();self.wfile.write(body)
            except (BrokenPipeError,ConnectionAbortedError,ConnectionResetError):pass
            except Exception as error:self.send_error(400,'Request failed',explain=str(error))
        def log_message(self,*args):pass
    print(f'http://127.0.0.1:{args.port}/',flush=True);ThreadingHTTPServer(('127.0.0.1',args.port),Handler).serve_forever()


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('action',choices=['generate','serve']);p.add_argument('--input',type=Path,default=Path('build/ridge-surface-half-budget-v2/ridge.sqlite3'));p.add_argument('--output',type=Path,default=Path('build/ridge-spline-prototype-v4'));p.add_argument('--port',type=int,default=61472);a=p.parse_args();dict(generate=generate,serve=serve)[a.action](a)
