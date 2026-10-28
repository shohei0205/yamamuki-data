"""良好なBスプラインを粗い曲面と差分へ分け、影響の小さい差分から削る。"""
import argparse
import base64
import functools
import gzip
import heapq
import json
import sqlite3
import time
import urllib.request
import zlib
from pathlib import Path
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from urllib.parse import urlparse,parse_qs
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import numpy as np
from scipy.ndimage import gaussian_filter
from scipy.sparse import csr_matrix,diags
from scipy.sparse.linalg import cg
from scipy.stats import qmc
from ridge_spline import design,SPAN
from compare_spline_codec import gaps,ungaps

KEEPS=[1.,.5,.25,.125,.0625]


def refine_matrix(coarse,fine):
    r=np.eye(coarse+5)
    current=coarse
    while current<fine:
        current*=2;p=np.arange(current+5);center=p//2+1;ix=np.column_stack((center-1,center,center+1))
        w=np.where((p%2==0)[:,None],[1.,6.,1.],[0.,4.,4.])/8
        r=sum(r[ix[:,k]]*w[:,k,None] for k in range(3))
    if current!=fine:raise ValueError('細かさは2倍ずつにしてください')
    return r


def blur_coefficients(c,sigma):
    if sigma==0:return c.copy()
    # 外側を線形に延長し、平面をならしても外周が曲がらないようにする。
    pad=int(np.ceil(4*sigma))+2
    x=np.pad(c,((0,0),(pad,pad)),mode='edge')
    for k in range(pad):
        x[:,pad-1-k]=c[:,0]-(k+1)*(c[:,1]-c[:,0]);x[:,-pad+k]=c[:,-1]+(k+1)*(c[:,-1]-c[:,-2])
    out=np.pad(x,((pad,pad),(0,0)),mode='edge')
    for k in range(pad):
        out[pad-1-k]=x[0]-(k+1)*(x[1]-x[0]);out[-pad+k]=x[-1]+(k+1)*(x[-1]-x[-2])
    return gaussian_filter(out,sigma,mode='nearest',truncate=4)[pad:-pad,pad:-pad]


def gram_1d(n):
    nodes,weights=np.polynomial.legendre.leggauss(4);t=(nodes+1)/2;step=SPAN/n
    b=np.column_stack(((1-t)**3,3*t**3-6*t*t+4,-3*t**3+3*t*t+3*t+1,t**3))/6
    db=np.column_stack((-3*(1-t)**2,9*t*t-12*t,-9*t*t+6*t+3,3*t*t))/6/step
    ix=(np.arange(n)[:,None,None]+np.arange(4)[None,None,:]+1);ix=np.broadcast_to(ix,(n,4,4)).reshape(-1)
    rows=np.repeat(np.arange(n*4),4);shape=(n*4,n+5)
    a=csr_matrix((np.tile(b,(n,1)).ravel(),(rows,ix)),shape=shape)
    d=csr_matrix((np.tile(db,(n,1)).ravel(),(rows,ix)),shape=shape);w=np.tile(weights*step/2,n)
    return (a.T@a.multiply(w[:,None])).toarray(),(d.T@d.multiply(w[:,None])).toarray()


def gram_neighbors(m,k,length):
    width=len(m);y,x=np.indices((width,width));y=y.ravel();x=x.ravel();dy,dx=np.meshgrid(np.arange(-3,4),np.arange(-3,4),indexing='ij');dy=dy.ravel();dx=dx.ravel()
    yy=y[:,None]+dy;xx=x[:,None]+dx;valid=(yy>=0)&(yy<width)&(xx>=0)&(xx<width)
    yy=np.clip(yy,0,width-1);xx=np.clip(xx,0,width-1)
    g=m[y[:,None],yy]*m[x[:,None],xx]+length**2*(k[y[:,None],yy]*m[x[:,None],xx]+m[y[:,None],yy]*k[x[:,None],xx])
    g[~valid]=0
    return (yy*width+xx).astype(np.int32),g


def prune(detail,keep,neighbors,gram,trace=None):
    coefficients=detail.ravel().astype(float);diagonal=gram[:,24]
    active=(abs(coefficients)>1e-7)&(diagonal>1e-20);initial=int(active.sum());wanted=int(np.floor(initial*keep))
    cost=coefficients**2*diagonal;versions=np.zeros(len(coefficients),np.int32)
    heap=list(zip(cost[active].tolist(),np.flatnonzero(active).tolist(),versions[active].tolist()));heapq.heapify(heap)
    removed=0;start=time.monotonic()
    while initial-removed>wanted:
        _,i,revision=heapq.heappop(heap)
        if not active[i] or revision!=versions[i]:continue
        active[i]=False;removed+=1
        if trace is not None:trace.append(i)
        ids=neighbors[i];mask=active[ids]&(gram[i]!=0);ids=ids[mask];g=gram[i,mask]
        cost[ids]+=2*coefficients[i]*coefficients[ids]*g;versions[ids]+=1
        for item in zip(cost[ids].tolist(),ids.tolist(),versions[ids].tolist()):heapq.heappush(heap,item)
        if removed%4096==0:
            print('差分を削減',removed,'/',initial-wanted,flush=True)
            if len(heap)>4*max(1,int(active.sum())):
                ix=np.flatnonzero(active);heap=list(zip(cost[ix].tolist(),ix.tolist(),versions[ix].tolist()));heapq.heapify(heap)
    return np.flatnonzero(active).astype('<u4'),dict(initialDetails=initial,retainedDetails=int(active.sum()),removedDetails=removed,pruneSeconds=time.monotonic()-start)


def refit_details(detail,indices,matrix):
    restricted=matrix[indices,:][:,indices].tocsr();rhs=(matrix@detail.ravel().astype(float))[indices]
    iterations=0
    def count(_):
        nonlocal iterations
        iterations+=1
    solution,status=cg(restricted,rhs,x0=detail.ravel()[indices].astype(float),M=diags(1/restricted.diagonal()),rtol=1e-8,atol=0.,maxiter=800,callback=count)
    if status!=0:raise ValueError(f'残した係数の再調整が収束しません: {status}')
    return solution.astype('<f4'),iterations


def derivatives(xy,n):
    q=(np.asarray(xy)+SPAN/2)*n/SPAN+2;cell=np.floor(q).astype(int);t=q-cell;step=SPAN/n
    def b(t):return np.column_stack(((1-t)**3,3*t**3-6*t*t+4,-3*t**3+3*t*t+3*t+1,t**3))/6
    def db(t):return np.column_stack((-3*(1-t)**2,9*t*t-12*t,-9*t*t+6*t+3,3*t*t))/6/step
    bx,by=b(t[:,0]),b(t[:,1]);dx,dy=db(t[:,0]),db(t[:,1]);cols=[];vx=[];vy=[]
    for j in range(4):
        for i in range(4):cols.append((cell[:,1]+j-1)*(n+5)+cell[:,0]+i-1);vx.append(dx[:,i]*by[:,j]);vy.append(bx[:,i]*dy[:,j])
    rr=np.repeat(np.arange(len(xy)),16);cc=np.column_stack(cols).ravel();shape=(len(xy),(n+5)**2)
    return csr_matrix((np.column_stack(vx).ravel(),(rr,cc)),shape=shape),csr_matrix((np.column_stack(vy).ravel(),(rr,cc)),shape=shape)


def reconstruct(base,indices,values,n):
    r=refine_matrix(len(base)-5,n);dense=r@base.astype(float)@r.T
    dense.ravel()[indices]+=values
    return dense.astype('<f4')


def generate(args):
    args.output.mkdir(parents=True,exist_ok=True);path=args.output/'pruned.sqlite3'
    if path.exists():raise ValueError('既存の結果を残すため新しい出力先を指定してください')
    n=args.n;coarse=max(8,n//8);r=refine_matrix(coarse,n);inverse=np.linalg.pinv(r)
    m,k=gram_1d(n);length=SPAN/n*2;neighbors,gram=gram_neighbors(m,k,length)
    matrix=csr_matrix((gram.ravel(),(np.repeat(np.arange((n+5)**2),49),neighbors.ravel())),shape=((n+5)**2,(n+5)**2));matrix.eliminate_zeros()
    axis=np.linspace(-SPAN/2,SPAN/2,257);x,y=np.meshgrid(axis,axis)
    query=np.vstack((np.column_stack((x.ravel(),y.ravel())),(qmc.Sobol(2,scramble=True,seed=319).random_base2(13)-.5)*SPAN))
    a=design(query,n);dx,dy=derivatives(query,n)
    with sqlite3.connect(args.input.resolve().as_uri()+'?mode=ro',uri=True) as source,sqlite3.connect(path) as db:
        db.execute('CREATE TABLE surfaces(region TEXT,keep REAL,info TEXT,base BLOB,indices BLOB,coefficients BLOB,PRIMARY KEY(region,keep)) WITHOUT ROWID');report=[]
        for region,raw,blob in source.execute('SELECT region,info,coefficients FROM surfaces WHERE n=? ORDER BY region',(n,)):
            info=json.loads(raw);original=np.frombuffer(zlib.decompress(blob),'<f4').reshape(n+5,n+5).astype(float)
            original_h=a@original.ravel();original_g=np.column_stack((dx@original.ravel(),dy@original.ravel()))
            for keep in KEEPS:
                sigma=.5*np.sqrt(1/keep-1) if not args.no_lpf else 0.;target=blur_coefficients(original,sigma)
                base=(inverse@target@inverse.T).astype('<f4');prediction=r@base.astype(float)@r.T;detail=(target-prediction).astype('<f4')
                print(region,'細部を残す割合',keep,'LPF',sigma*SPAN/n,flush=True)
                ix,detail_info=prune(detail,keep,neighbors,gram);values=detail.ravel()[ix]
                if args.continuous and keep<1:
                    # 孤立した細格子基底を残さず、予算内の連続した粗い基底へ投影する。
                    budget=coarse**2+10*coarse+25+int(detail_info['initialDetails']*keep)
                    fit_n=8
                    while (fit_n*2+5)**2<=budget and fit_n*2<=n:fit_n*=2
                    fit_r=refine_matrix(fit_n,n);fit_inverse=np.linalg.pinv(fit_r)
                    base=(fit_inverse@target@fit_inverse.T).astype('<f4')
                    ix=np.empty(0,dtype='<u4');values=np.empty(0,dtype='<f4')
                    detail_info.update(retainedDetails=0,removedDetails=detail_info['initialDetails'])
                before=reconstruct(base,ix,values,n);before_rmse=float(np.sqrt(np.mean((a@before.ravel()-a@target.ravel())**2)))
                refit_iterations=0
                if keep<1 and not args.continuous:values,refit_iterations=refit_details(detail,ix,matrix)
                restored=reconstruct(base,ix,values,n);h=a@restored.ravel();gradient=np.column_stack((dx@restored.ravel(),dy@restored.ravel()))
                target_h=a@target.ravel();target_g=np.column_stack((dx@target.ravel(),dy@target.ravel()))
                normal=np.column_stack((-gradient[:,0],np.ones(len(query)),-gradient[:,1]));normal/=np.linalg.norm(normal,axis=1)[:,None]
                reference_normal=np.column_stack((-original_g[:,0],np.ones(len(query)),-original_g[:,1]));reference_normal/=np.linalg.norm(reference_normal,axis=1)[:,None]
                angle=np.degrees(np.arccos(np.clip(np.sum(normal*reference_normal,axis=1),-1,1)))
                bb=zlib.compress(base.tobytes(),6);ib=zlib.compress(gaps(ix),6);cb=zlib.compress(values.tobytes(),6)
                row=dict(info);row.update(detail_info);row.update(lpfEnabled=not args.no_lpf,refit=True,refitIterations=refit_iterations,pruningRmseBeforeRefit=before_rmse);row.update(region=region,keep=keep,pruning=True,continuousBasis=args.continuous,baseN=len(base)-5,baseCount=int(base.size),retainedCount=int(base.size+len(ix)),sourceCoefficientCount=int(original.size),originalBytes=len(blob),cumulativeBytes=len(bb)+len(ib)+len(cb),baseBytes=len(bb),indexBytes=len(ib),detailBytes=len(cb),sigmaMeters=float(sigma*SPAN/n),heightRmse=float(np.sqrt(np.mean((h-original_h)**2))),heightMax=float(np.max(abs(h-original_h))),gradientRmse=float(np.sqrt(np.mean(np.sum((gradient-original_g)**2,axis=1)))),normalP95Degrees=float(np.percentile(angle,95)),normalMaxDegrees=float(angle.max()),pruningRmse=float(np.sqrt(np.mean((h-target_h)**2))),pruningGradientRmse=float(np.sqrt(np.mean(np.sum((gradient-target_g)**2,axis=1)))),lpfRmse=float(np.sqrt(np.mean((target_h-original_h)**2))),gradientLengthMeters=length,evaluationCount=len(query),indexCodec='gap-varint-zlib6',coefficientCodec='float32-zlib6')
                db.execute('INSERT INTO surfaces VALUES(?,?,?,?,?,?)',(region,keep,json.dumps(row),bb,ib,cb));db.commit();report.append(row);print(json.dumps(row),flush=True)
        (args.output/'report.json').write_text(json.dumps(report,indent=2),encoding='utf-8')


def serve(args):
    @functools.lru_cache(maxsize=18)
    def packet(region,keep,lpf):
        output=args.output if lpf else args.no_lpf_output
        with sqlite3.connect((output/'pruned.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as db:raw,bb,ib,cb=db.execute('SELECT info,base,indices,coefficients FROM surfaces WHERE region=? AND keep=?',(region,keep)).fetchone()
        info=json.loads(raw);info['lpfEnabled']=lpf;n=info['n'];base=np.frombuffer(zlib.decompress(bb),'<f4').reshape(info['baseN']+5,-1);ix=ungaps(zlib.decompress(ib));values=np.frombuffer(zlib.decompress(cb),'<f4');dense=reconstruct(base,ix,values,n)
        with sqlite3.connect(args.input.resolve().as_uri()+'?mode=ro',uri=True) as db:ref_raw,ref_blob=db.execute('SELECT info,coefficients FROM surfaces WHERE region=? AND n=?',(region,n)).fetchone()
        ref_info=json.loads(ref_raw);enc=lambda b:base64.b64encode(b).decode()
        points=[]
        for size,indices,c in [(info['baseN'],np.arange(base.size),base.ravel()),(n,ix,dense.ravel()[ix])]:
            yy,xx=np.divmod(indices,size+5);xy=np.column_stack(((xx-2)*SPAN/size-SPAN/2,(yy-2)*SPAN/size-SPAN/2));inside=np.all(abs(xy)<=SPAN/2,axis=1);points.extend(np.column_stack((xy[inside],c[inside])).tolist())
        model=dict(info=info,size=n+5,halo=2,span=SPAN,summit=ref_info['summit'],coefficients=enc(dense.tobytes()),controlPoints=enc(np.asarray(points,'<f4').tobytes()),controlCount=len(points))
        model['reference']=dict(info=ref_info,size=n+5,halo=2,span=SPAN,summit=ref_info['summit'],coefficients=enc(zlib.decompress(ref_blob)))
        return gzip.compress(json.dumps(model).encode(),1)
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            try:
                u=urlparse(self.path);q=parse_qs(u.query);encoding=None
                if u.path=='/api/pruned-spline':body=packet(q.get('region',['yari'])[0],float(q.get('keep',['0.25'])[0]),q.get('lpf',['1'])[0]!='0');mime='application/json';encoding='gzip'
                elif u.path=='/':
                    with urllib.request.urlopen('http://127.0.0.1:61472/') as response:html=response.read().decode()
                    html=html.replace('<option value="ridgeSpline" selected>','<option value="prunedSpline" selected>Bスプライン・差分係数の削減</option><option value="ridgeSpline">')
                    html=html.replace('<button id="ridgeFocus">','<label><input type="checkbox" id="pruneLPF" checked>削減量に応じて全域をならす</label><label>残す細部係数 <select id="pruneKeep"><option value="1">100%（基準）</option><option value="0.5">50%</option><option value="0.25" selected>25%</option><option value="0.125">12.5%</option><option value="0.0625">6.25%</option></select></label><button id="ridgeFocus">')
                    html=html.replace('<script src="/lpf_curve_compare.js">','<script>window.prunedSplineEnabled=true;</script><script src="/lpf_curve_compare.js">')
                    html=html.replace('id="splineControlPoints" checked','id="splineControlPoints"').replace('Bスプライン制御点','保存係数の位置').replace('近似の入力点','元Bスプラインの入力点')
                    if args.continuous:
                        html=html.replace('Bスプライン・差分係数の削減','Bスプライン・連続した粗い基底').replace('残す細部係数','係数予算の目安').replace('id="pruneLPF" checked','id="pruneLPF" disabled')
                    a=html.index('<div id="note">');b=html.index('</div>',a)
                    html=html[:a]+'<div id="note">左：元の「制御点から一続きのBスプライン」（125 m間隔）。右：粗い曲面＋残した差分係数。高さと傾きへの影響が小さい係数から削り、近傍の優先度は削除ごとに更新し、残った係数も再調整します。割合は細部係数の数で、容量比ではありません。チェックONでは低品質ほど全域共通のLPFも強め、OFFでは係数の削除だけを比較できます。保存量・形状差・法線の差を表示します。元データは保持しています。</div>'+html[b+6:]
                    if args.continuous:
                        start=html.index('<div id="note">');end=html.index('</div>',start)
                        html=html[:start]+'<div id="note">左：元のBスプライン。右：予算内の連続した粗いBスプライン基底へ再近似。細格子の係数を孤立して残す方法をやめ、ドット状の局所的な凹凸を避けます。予算は上限の目安で、格子は2倍刻みのため複数設定が同じ細かさになります。全域LPFはOFF固定。元の削減方式は61476に保持しています。</div>'+html[end+6:]
                    body=html.encode();mime='text/html; charset=utf-8'
                else:
                    with urllib.request.urlopen('http://127.0.0.1:61472'+self.path,timeout=180) as response:body=response.read();mime=response.headers['Content-Type'];encoding=response.headers.get('Content-Encoding')
                self.send_response(200);self.send_header('Content-Type',mime);self.send_header('Content-Length',str(len(body)));self.send_header('Cache-Control','no-cache')
                if encoding:self.send_header('Content-Encoding',encoding)
                self.end_headers();self.wfile.write(body)
            except (BrokenPipeError,ConnectionAbortedError,ConnectionResetError):pass
            except Exception as e:self.send_error(400,'Request failed',explain=str(e))
        def log_message(self,*args):pass
    print(f'http://127.0.0.1:{args.port}/',flush=True);ThreadingHTTPServer(('127.0.0.1',args.port),Handler).serve_forever()

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('action',choices=['generate','serve']);p.add_argument('--input',type=Path,default=Path('build/ridge-spline-prototype-v4/spline.sqlite3'));p.add_argument('--output',type=Path,default=Path('build/spline-pruning-v2'));p.add_argument('--no-lpf',action='store_true');p.add_argument('--continuous',action='store_true');p.add_argument('--no-lpf-output',type=Path,default=Path('build/spline-pruning-no-lpf-v1'));p.add_argument('--n',type=int,default=256);p.add_argument('--port',type=int,default=61476);a=p.parse_args();dict(generate=generate,serve=serve)[a.action](a)
