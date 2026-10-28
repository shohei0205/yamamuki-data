"""局所領域だけを階層細分化し、三次Bスプラインの連続性を保つ比較。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import argparse,base64,functools,gzip,json,sqlite3,urllib.request,zlib,time
from pathlib import Path
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from urllib.parse import urlparse,parse_qs
import numpy as np
from scipy.sparse import csr_matrix,csc_matrix,eye,kron,hstack
from scipy.sparse.linalg import lsmr
from scipy.ndimage import map_coordinates,gaussian_filter,binary_opening
from nonuniform_spline import cardinal
from prune_ridge_spline import refine_matrix,KEEPS,SPAN,blur_coefficients
LOCAL_KEEPS=KEEPS+[.03125,.015625,.0078125]
from compare_spline_codec import gaps,ungaps

def inside_basis(domain):
    n=len(domain);ix=np.arange(n+5);lo=np.clip(ix-4,0,n);hi=np.clip(ix,0,n)
    integral=np.pad(domain.astype(int).cumsum(0).cumsum(1),((1,0),(1,0)))
    count=integral[hi[:,None],hi]-integral[lo[:,None],hi]-integral[hi[:,None],lo]+integral[lo[:,None],lo]
    area=(hi-lo)[:,None]*(hi-lo)
    return ((count==area)&(area>0)).ravel()

def truncate(transform,refinement,selected):
    expanded=(refinement@transform).tocsr().multiply((~selected)[:,None]).tocsr();expanded.eliminate_zeros()
    live=np.asarray(abs(expanded).sum(axis=0)).ravel()>1e-14
    ids=np.flatnonzero(selected);detail=csr_matrix((np.ones(len(ids)),(ids,np.arange(len(ids)))),shape=(expanded.shape[0],len(ids)))
    return hstack((expanded[:,live],detail),format='csr'),live,ids

def choose_domain(error,n,parent,allowance,graded=False,density=None):
    y,x=np.indices((n,n));q=np.array([(y+.5)/n*(len(error)-1),(x+.5)/n*(len(error)-1)])
    score=map_coordinates(error**2,q,order=1,prefilter=False)
    domain=np.zeros((n,n),bool);available=np.repeat(np.repeat(parent,2,0),2,1)
    if graded:
        # 残差を約2kmの範囲で評価し、四角い区画の選別をやめる。
        score=gaussian_filter(score,sigma=max(1,n/(8 if density is not None else 16)),mode='nearest')
        if density is not None:
            mean=max(float(density.mean()),1e-12)
            score/=np.maximum(.5,density/mean)**2
        radius=2;yy,xx=np.mgrid[-radius:radius+1,-radius:radius+1];disk=xx*xx+yy*yy<=radius*radius
        low=0.;high=float(score.max())+1e-12;best=domain
        for _ in range(30):
            threshold=(low+high)/2;trial=binary_opening((score>=threshold)&available,structure=disk,border_value=0)
            if inside_basis(trial).sum()<=allowance:best=trial;high=threshold
            else:low=threshold
        return best
    candidates=[]
    for y in range(0,n,8):
        for x in range(0,n,8):
            block=score[y:y+8,x:x+8];valid=available[y:y+8,x:x+8]
            if valid.any():candidates.append((float((block*valid).sum()),y,x))
    for _,y,x in sorted(candidates,reverse=True):
        trial=domain.copy();trial[max(0,y-3):min(n,y+11),max(0,x-3):min(n,x+11)]=True;trial&=available
        if inside_basis(trial).sum()<=allowance:domain=trial
    return domain

def fit_coefficients(a,h):
    norm=np.sqrt(np.asarray(a.multiply(a).sum(axis=0)).ravel());live=norm>1e-12
    if not live.all():raise ValueError('台が評価範囲に届かない基底があります')
    scaled=a.multiply(1/norm).tocsr();result=lsmr(scaled,h,damp=.01,atol=1e-5,btol=1e-5,maxiter=2000)
    if result[1] not in (0,1,2,4,5):raise ValueError(f'階層曲面の近似が収束しません: {result[1]}')
    return result[0]/norm,int(result[2])

def basis_density(labels,n):
    density=np.zeros((n,n))
    for level,index in labels:
        y,x=divmod(index,level+5);xx=int(np.clip((x-2)*n/level,0,n-1));yy=int(np.clip((y-2)*n/level,0,n-1));density[yy,xx]+=1
    return gaussian_filter(density,sigma=max(1,n/8),mode='reflect')

def fit(h,budget,graded=False,balanced=False):
    axis=np.linspace(-SPAN/2,SPAN/2,len(h));n=8;one=csr_matrix(cardinal(n,axis));evaluation=kron(one,one,format='csr')
    used=np.asarray(evaluation.sum(axis=0)).ravel()>0;transform=eye((n+5)**2,format='csr')[:,used];labels=[(n,int(i)) for i in np.flatnonzero(used)]
    history=[dict(n=8,indices=np.flatnonzero(used).tolist())]
    coefficients,iterations=fit_coefficients(evaluation@transform,h.ravel());error=(evaluation@transform@coefficients).reshape(h.shape)-h;parent=np.ones((n,n),bool);stages=[]
    max_fine=min(256,2**int(np.ceil(np.log2(np.sqrt(budget))))) if graded else 256
    for fine in [16,32,64,128,256]:
        cap=max(len(labels),int(budget*(fine/max_fine)**.25)) if graded else max(len(labels),int(budget*np.sqrt(fine/256)))
        domain=choose_domain(error,fine,parent,max(0,cap-len(labels)),graded=graded,density=basis_density(labels,fine) if balanced else None) if fine<=max_fine else np.zeros((fine,fine),bool)
        selected=inside_basis(domain)
        refinement=kron(csr_matrix(refine_matrix(n,fine)),csr_matrix(refine_matrix(n,fine)),format='csr')
        transform,live,ids=truncate(transform,refinement,selected)
        labels=[label for label,keep in zip(labels,live) if keep]+[(fine,int(i)) for i in ids]
        one=csr_matrix(cardinal(fine,axis));evaluation=kron(one,one,format='csr');a=evaluation@transform
        used=np.asarray(a.multiply(a).sum(axis=0)).ravel()>1e-24;transform=transform[:,used];a=a[:,used];labels=[label for label,keep in zip(labels,used) if keep]
        coefficients,it=fit_coefficients(a,h.ravel());iterations+=it;error=(a@coefficients).reshape(h.shape)-h
        history.append(dict(n=fine,gaps=base64.b64encode(gaps(ids.astype('<u4'))).decode(),used=base64.b64encode(np.packbits(used).tobytes()).decode(),usedCount=len(used)))
        stages.append(dict(n=fine,coefficients=len(labels),localCells=int(domain.sum()),rmse=float(np.sqrt(np.mean(error**2))),iterations=it))
        n=fine;parent=domain
    return labels,coefficients,transform,stages,iterations,history


def generate(a):
    a.output.mkdir(parents=True,exist_ok=True);path=a.output/'local.sqlite3'
    if path.exists() and not a.extend:raise ValueError('既存データを残すため新しい出力先か --extend を指定してください')
    x=np.linspace(-SPAN/2,SPAN/2,257);u=cardinal(256,x);reports=[]
    with sqlite3.connect(a.input) as source,sqlite3.connect(path) as db:
        db.execute('CREATE TABLE IF NOT EXISTS surfaces(region TEXT,keep REAL,info TEXT,structure BLOB,coefficients BLOB,PRIMARY KEY(region,keep)) WITHOUT ROWID')
        for region,raw,blob in source.execute('SELECT region,info,coefficients FROM surfaces WHERE n=256'):
            original=np.frombuffer(zlib.decompress(blob),'<f4').reshape(261,261);h=u@original@u.T
            for keep in LOCAL_KEEPS:
                if db.execute('SELECT 1 FROM surfaces WHERE region=? AND keep=?',(region,keep)).fetchone():continue
                start=time.monotonic();budget=1369+int(67081*keep)
                balanced=a.allocation=='balanced' and keep<=.125
                common_sigma=SPAN/np.sqrt(budget)*.6 if balanced else 0.
                # 保存する基底密度にかかわらず、全域の細部尺度をそろえる。
                target_coefficients=blur_coefficients(original.astype(float),common_sigma/125)
                target=u@target_coefficients@u.T
                labels,values,t,stages,iterations,history=fit(target,budget,graded=a.allocation in ('graded','balanced') and keep<=.125,balanced=balanced)
                packed=json.dumps(history,separators=(',',':')).encode()
                vb=zlib.compress(np.asarray(values,'<f4').tobytes(),6);sb=zlib.compress(packed,6)
                dense=(t@np.asarray(values,'<f4')).reshape(261,261).astype('<f4');error=u@dense@u.T-h
                info=json.loads(raw);info.update(pruning=True,continuousBasis=True,localBasis=True,allocation=a.allocation if keep<=.125 else 'block',commonFilterMeters=float(common_sigma),commonFilterRmse=float(np.sqrt(np.mean((target-h)**2))),fitTargetRmse=float(np.sqrt(np.mean((u@dense@u.T-target)**2))),keep=keep,region=region,baseN=256,baseCount=len(values),retainedCount=len(values),retainedDetails=0,lpfEnabled=False,refit=True,cumulativeBytes=len(sb)+len(vb),structureBytes=len(sb),coefficientBytes=len(vb),originalBytes=len(blob),heightRmse=float(np.sqrt(np.mean(error**2))),heightMax=float(abs(error).max()),normalP95Degrees=0.,sigmaMeters=0.,lpfRmse=0.,pruningRmse=float(np.sqrt(np.mean(error**2))),iterations=iterations,stages=stages,budget=budget,seconds=time.monotonic()-start)
                db.execute('INSERT INTO surfaces VALUES(?,?,?,?,?)',(region,keep,json.dumps(info),sb,vb));db.commit();reports.append(info);print(region,keep,len(values),len(sb)+len(vb),info['heightRmse'],info['seconds'],flush=True)
    with sqlite3.connect(path) as db:reports=[json.loads(row[0]) for row in db.execute('SELECT info FROM surfaces ORDER BY region,keep DESC')]
    (a.output/'report.json').write_text(json.dumps(reports,indent=2),encoding='utf-8')

def unpack(blob):
    history=json.loads(zlib.decompress(blob));n=8;ids=history[0]['indices'];t=eye(169,format='csr')[:,ids];labels=[(8,int(i)) for i in ids]
    for layer in history[1:]:
        fine=layer['n'];ids=ungaps(base64.b64decode(layer['gaps']));selected=np.zeros((fine+5)**2,bool);selected[ids]=True
        r=kron(csr_matrix(refine_matrix(n,fine)),csr_matrix(refine_matrix(n,fine)),format='csr');t,live,new=truncate(t,r,selected)
        labels=[label for label,keep in zip(labels,live) if keep]+[(fine,int(i)) for i in new]
        used=np.unpackbits(np.frombuffer(base64.b64decode(layer['used']),'u1'))[:layer['usedCount']].astype(bool);t=t[:,used];labels=[label for label,keep in zip(labels,used) if keep];n=fine
    return t,labels


def serve(a):
    @functools.lru_cache(maxsize=32)
    def packet(region,keep):
        with sqlite3.connect(a.output/'local.sqlite3') as db:raw,sb,vb=db.execute('SELECT info,structure,coefficients FROM surfaces WHERE region=? AND keep=?',(region,keep)).fetchone()
        info=json.loads(raw);values=np.frombuffer(zlib.decompress(vb),'<f4');transform,labels=unpack(sb);dense=(transform@values).reshape(261,261).astype('<f4');points=[]
        # 基底位置は階層ごとの中心。高さは合成曲面上の値として表示する。
        from ridge_spline import design
        for n,index in labels:
            y,x=divmod(index,n+5);px=(x-2)*SPAN/n-SPAN/2;py=(y-2)*SPAN/n-SPAN/2
            if abs(px)<=SPAN/2 and abs(py)<=SPAN/2:points.append([px,py])
        xy=np.asarray(points);points=np.column_stack((xy,design(xy,256)@dense.ravel())).astype('<f4');ref_info=None
        with sqlite3.connect(a.input) as db:refraw,refblob=db.execute('SELECT info,coefficients FROM surfaces WHERE region=? AND n=256',(region,)).fetchone()
        ref=json.loads(refraw);enc=lambda v:base64.b64encode(v).decode()
        model=dict(info=info,size=261,halo=2,span=SPAN,summit=ref['summit'],coefficients=enc(dense.tobytes()),controlPoints=enc(points.tobytes()),controlCount=len(points),reference=dict(info=ref,size=261,halo=2,span=SPAN,summit=ref['summit'],coefficients=enc(zlib.decompress(refblob))))
        return gzip.compress(json.dumps(model).encode(),1)
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            try:
                path=urlparse(self.path);q=parse_qs(path.query);encoding=None
                if path.path=='/api/pruned-spline':body=packet(q.get('region',['yari'])[0],float(q.get('keep',['.0625'])[0]));mime='application/json';encoding='gzip'
                elif path.path=='/':
                    with urllib.request.urlopen('http://127.0.0.1:61477/') as r:s=r.read().decode()
                    s=s.replace('Bスプライン・連続した粗い基底','Bスプライン・局所階層細分化')
                    s=s.replace('<option value="0.0625">6.25%</option>','<option value="0.0625">6.25%</option><option value="0.03125">3.125%（6.25%の1/2）</option><option value="0.015625">1.5625%（1/4）</option><option value="0.0078125">0.78125%（1/8）</option>')
                    start=s.index('<div id="note">');end=s.index('</div>',start)
                    s=s[:start]+'<div id="note">左：元の曲面。右：低予算では全域の細部尺度をそろえ、周囲約4kmの基底密度が高い場所への集中も抑えます。局所的な詳細さの突出を抑えるため、細部は全体に丸まります。Tスプラインは使用していません。階層内部は2倍刻みの格子ですが、分割線を領域全体には延ばしません。保存係数の位置は基底中心の配置を曲面上に表示します。階層ごとの局所選択範囲と高さ係数を保存し、表示時に同じ曲面へ正確に展開します。</div>'+s[end+6:];body=s.encode();mime='text/html; charset=utf-8'
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
    p=argparse.ArgumentParser();p.add_argument('action',choices=['generate','serve']);p.add_argument('--input',type=Path,default=Path('build/ridge-spline-prototype-v4/spline.sqlite3'));p.add_argument('--output',type=Path,default=Path('build/local-spline-v7'));p.add_argument('--port',type=int,default=61479);p.add_argument('--extend',action='store_true');p.add_argument('--allocation',choices=['block','graded','balanced'],default='balanced');a=p.parse_args();dict(generate=generate,serve=serve)[a.action](a)
