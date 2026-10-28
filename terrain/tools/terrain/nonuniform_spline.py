"""誤差に応じて不等間隔ノットを配置する、一続きのBスプライン。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import argparse,base64,functools,gzip,json,sqlite3,urllib.request,zlib
from pathlib import Path
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from urllib.parse import urlparse,parse_qs
import numpy as np
from scipy.interpolate import BSpline
from scipy.ndimage import gaussian_filter1d
from prune_ridge_spline import KEEPS
SPAN=32000.

def cardinal(n,x):
    q=(x+SPAN/2)*n/SPAN+2;cell=np.floor(q).astype(int);t=q-cell
    b=np.column_stack(((1-t)**3,3*t**3-6*t*t+4,-3*t**3+3*t*t+3*t+1,t**3))/6
    a=np.zeros((len(x),n+5))
    for j in range(4):a[np.arange(len(x)),cell+j-1]=b[:,j]
    return a

def knots_for(density,count,x):
    # 一点への極端な密集を避け、ただし等間隔には固定しない。
    d=np.maximum(gaussian_filter1d(density,3),0);d=.25+.75*d/max(float(d.mean()),1e-12);d=np.clip(d,.25,3.)
    cdf=np.r_[0,np.cumsum((d[:-1]+d[1:])/2*np.diff(x))];cdf/=cdf[-1]
    interior=np.interp(np.arange(1,count-3)/(count-3),cdf,x)
    return np.r_[np.repeat(x[0],4),interior,np.repeat(x[-1],4)]

def fit_surface(h,x,count):
    curvature=np.abs(np.gradient(np.gradient(h,axis=0),axis=0))+np.abs(np.gradient(np.gradient(h,axis=1),axis=1))
    dx=np.sqrt(np.mean(curvature**2,axis=0));dy=np.sqrt(np.mean(curvature**2,axis=1))
    best=None
    for iteration in range(3):
        tx=knots_for(dx,count,x);ty=knots_for(dy,count,x)
        bx=BSpline.design_matrix(x,tx,3).toarray();by=BSpline.design_matrix(x,ty,3).toarray()
        c=np.linalg.pinv(by,rcond=1e-4)@h@np.linalg.pinv(bx,rcond=1e-4).T;error=by@c@bx.T-h;rmse=float(np.sqrt(np.mean(error**2)))
        if best is None or rmse<best[0]:best=(rmse,tx,ty,c)
        dx=.5*dx+.5*np.sqrt(np.mean(error**2,axis=0));dy=.5*dy+.5*np.sqrt(np.mean(error**2,axis=1))
    return best

def generate(a):
    a.output.mkdir(parents=True,exist_ok=True);path=a.output/'nonuniform.sqlite3'
    if path.exists():raise ValueError('既存データを残すため別の出力先を指定してください')
    x=np.linspace(-SPAN/2,SPAN/2,513);u=cardinal(256,x);inv=np.linalg.pinv(u);report=[]
    with sqlite3.connect(a.input) as source,sqlite3.connect(path) as db:
        db.execute('CREATE TABLE surfaces(region TEXT,keep REAL,info TEXT,knotsX BLOB,knotsY BLOB,coefficients BLOB,PRIMARY KEY(region,keep)) WITHOUT ROWID')
        for region,raw,blob in source.execute('SELECT region,info,coefficients FROM surfaces WHERE n=256'):
            original=np.frombuffer(zlib.decompress(blob),'<f4').reshape(261,261);h=u@original@u.T
            for keep in KEEPS:
                count=max(8,int(np.sqrt(1369+int(67081*keep))));error,tx,ty,c=fit_surface(h,x,count)
                tx=tx.astype('<f4');ty=ty.astype('<f4');c=c.astype('<f4')
                bx=BSpline.design_matrix(x,tx,3).toarray();by=BSpline.design_matrix(x,ty,3).toarray();fitted=by@c@bx.T
                dense=inv@fitted@inv.T;display=u@dense@u.T
                bb=[zlib.compress(v.tobytes(),6) for v in (tx,ty,c)]
                info=json.loads(raw);info.update(pruning=True,continuousBasis=True,nonuniformBasis=True,keep=keep,region=region,baseN=256,baseCount=int(c.size),retainedDetails=0,retainedCount=int(c.size),lpfEnabled=False,refit=True,cumulativeBytes=sum(map(len,bb)),originalBytes=len(blob),heightRmse=float(np.sqrt(np.mean((fitted-h)**2))),heightMax=float(abs(fitted-h).max()),normalP95Degrees=0.,sigmaMeters=0.,lpfRmse=0.,pruningRmse=error,displayRmse=float(np.sqrt(np.mean((display-fitted)**2))),displayMax=float(abs(display-fitted).max()),controlSide=count,nonuniformIntervalsX=np.diff(np.unique(tx)).tolist(),nonuniformIntervalsY=np.diff(np.unique(ty)).tolist())
                db.execute('INSERT INTO surfaces VALUES(?,?,?,?,?,?)',(region,keep,json.dumps(info),*bb));db.commit();report.append(info);print(region,keep,count,info['cumulativeBytes'],info['heightRmse'],info['displayMax'],flush=True)
    (a.output/'report.json').write_text(json.dumps(report,indent=2),encoding='utf-8')

def serve(a):
    x=np.linspace(-SPAN/2,SPAN/2,513);u=cardinal(256,x);inv=np.linalg.pinv(u)
    @functools.lru_cache(maxsize=18)
    def packet(region,keep):
        with sqlite3.connect(a.output/'nonuniform.sqlite3') as db:raw,xb,yb,cb=db.execute('SELECT info,knotsX,knotsY,coefficients FROM surfaces WHERE region=? AND keep=?',(region,keep)).fetchone()
        info=json.loads(raw);tx=np.frombuffer(zlib.decompress(xb),'<f4');ty=np.frombuffer(zlib.decompress(yb),'<f4');c=np.frombuffer(zlib.decompress(cb),'<f4').reshape(info['controlSide'],-1)
        bx=BSpline.design_matrix(x,tx,3).toarray();by=BSpline.design_matrix(x,ty,3).toarray();dense=(inv@(by@c@bx.T)@inv.T).astype('<f4')
        with sqlite3.connect(a.input) as db:refraw,refblob=db.execute('SELECT info,coefficients FROM surfaces WHERE region=? AND n=256',(region,)).fetchone()
        ref=json.loads(refraw);enc=lambda v:base64.b64encode(v).decode()
        gx=np.array([tx[i+1:i+4].mean() for i in range(len(c))]);gy=np.array([ty[i+1:i+4].mean() for i in range(len(c))]);xx,yy=np.meshgrid(gx,gy);points=np.column_stack((xx.ravel(),yy.ravel(),c.ravel())).astype('<f4')
        model=dict(info=info,size=261,halo=2,span=SPAN,summit=ref['summit'],coefficients=enc(dense.tobytes()),controlPoints=enc(points.tobytes()),controlCount=len(points),reference=dict(info=ref,size=261,halo=2,span=SPAN,summit=ref['summit'],coefficients=enc(zlib.decompress(refblob))))
        return gzip.compress(json.dumps(model).encode(),1)
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            try:
                path=urlparse(self.path);q=parse_qs(path.query);encoding=None
                if path.path=='/api/pruned-spline':body=packet(q.get('region',['yari'])[0],float(q.get('keep',['.0625'])[0]));mime='application/json';encoding='gzip'
                elif path.path=='/':
                    with urllib.request.urlopen('http://127.0.0.1:61477/') as r:s=r.read().decode()
                    s=s.replace('Bスプライン・連続した粗い基底','Bスプライン・不等間隔ノット')
                    start=s.index('<div id="note">');end=s.index('</div>',start)
                    s=s[:start]+'<div id="note">左：元の曲面。右：地形の曲率と再現誤差から不等間隔に配置したノットのBスプライン。等間隔ではありませんが、縦横の直交配列は残ります。保存係数の位置で配置を確認できます。ノット列と高さ係数を保存し、表示時だけGPU用の格子へ再構成します。法線角度差は未計測。GPU表示への再近似誤差も表示します。</div>'+s[end+6:];body=s.encode();mime='text/html; charset=utf-8'
                else:
                    with urllib.request.urlopen('http://127.0.0.1:61472'+self.path,timeout=180) as r:body=r.read();mime=r.headers['Content-Type'];encoding=r.headers.get('Content-Encoding')
                self.send_response(200);self.send_header('Content-Type',mime);self.send_header('Content-Length',str(len(body)));self.send_header('Cache-Control','no-cache')
                if encoding:self.send_header('Content-Encoding',encoding)
                self.end_headers();self.wfile.write(body)
            except (BrokenPipeError,ConnectionResetError,ConnectionAbortedError):pass
            except Exception as e:self.send_error(400,'Request failed',explain=str(e))
        def log_message(self,*args):pass
    print(f'http://127.0.0.1:{a.port}/',flush=True);ThreadingHTTPServer(('127.0.0.1',a.port),Handler).serve_forever()

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('action',choices=['generate','serve']);p.add_argument('--input',type=Path,default=Path('build/ridge-spline-prototype-v4/spline.sqlite3'));p.add_argument('--output',type=Path,default=Path('build/nonuniform-spline-v2'));p.add_argument('--port',type=int,default=61478);a=p.parse_args();dict(generate=generate,serve=serve)[a.action](a)
