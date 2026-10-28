"""自由配置の高さ点から移動最小二乗で曲面を復元する比較。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import argparse,base64,functools,gzip,json,sqlite3,time,urllib.request,zlib
from pathlib import Path
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from urllib.parse import urlparse,parse_qs
import numpy as np
from scipy.spatial import cKDTree
from scipy.sparse import csr_matrix
from scipy.sparse.linalg import lsmr
from scipy.ndimage import gaussian_filter,map_coordinates,spline_filter
from scipy.stats import qmc
from ridge_spline import design,SPAN
from adaptive_allocation_surface import distribute
COUNTS=[128,256,512,2048,8192,16384,32768]

class LocalSupport:
    """保存XYだけから、滑らかに変化する近傍半径を作る。"""
    def __init__(self,points,scale=1.25,pre_smooth=True,floor_factor=2,grid_n=128):
        self.n=grid_n;self.spacing=SPAN/np.sqrt(len(points));axis=np.linspace(-SPAN/2,SPAN/2,self.n+1);x,y=np.meshgrid(axis,axis);q=np.column_stack((x.ravel(),y.ravel()))
        distance=cKDTree(points).query(q,k=min(12,len(points)))[0][:,-1]
        raw=np.maximum(floor_factor*self.spacing,scale*distance).reshape(self.n+1,self.n+1)
        log_radius=np.log(raw)
        if pre_smooth:log_radius=gaussian_filter(log_radius,1,mode='nearest')
        self.log_coefficients=spline_filter(log_radius,order=3,mode='nearest')
    def __call__(self,q):
        coordinates=((np.asarray(q)/SPAN+.5)*self.n).T[[1,0]]
        return np.exp(map_coordinates(self.log_coefficients,coordinates,order=3,prefilter=False,mode='nearest'))

def operator(points,query,radius,curvature=1e-3,tree=None):
    """二次MLSを評価する。tree指定時はpointsと同一座標の近傍木を渡す。"""
    tree=cKDTree(points) if tree is None else tree
    if tree.n!=len(points) or tree.m!=2:raise ValueError('MLSの近傍木と点数が一致しません')
    rows=[];columns=[];values=[]
    for start in range(0,len(query),128):
        q=query[start:start+128];radii=radius(q) if callable(radius) else np.full(len(q),radius);groups=tree.query_ball_point(q,radii);width=max(map(len,groups))
        if min(map(len,groups))<6:raise ValueError('MLSの近傍点が6点未満です')
        ids=np.zeros((len(q),width),int);mask=np.zeros_like(ids,bool)
        for k,g in enumerate(groups):ids[k,:len(g)]=g;mask[k,:len(g)]=True
        d=(points[ids]-q[:,None,:])/radii[:,None,None];u=d[:,:,0];v=d[:,:,1];r=np.linalg.norm(d,axis=2);s=np.maximum(0,1-r)
        w=s**4*(4*r+1)*mask;p=np.stack((np.ones_like(u),u,v,u*u,u*v,v*v),axis=2)
        normal=np.einsum('bki,bk,bkj->bij',p,w,p,optimize=True)
        # 一次面はほぼ厳密に保ち、疎な領域の曲率の暴走を抑える。
        normal+=np.diag([1e-10,1e-10,1e-10,curvature,curvature,curvature])
        rhs=np.zeros((len(q),6,1));rhs[:,0,0]=1
        alpha=np.linalg.solve(normal,rhs)[:,:,0];weights=w*np.einsum('bki,bi->bk',p,alpha)
        if not np.isfinite(weights).all():raise ValueError('MLSの係数が有限ではありません')
        rows.append(np.broadcast_to(np.arange(start,start+len(q))[:,None],ids.shape)[mask]);columns.append(ids[mask]);values.append(weights[mask])
    return csr_matrix((np.concatenate(values),(np.concatenate(rows),np.concatenate(columns))),shape=(len(query),len(points)))

def reference_values(original,xy):return design(xy,256)@original.ravel()

def optimize(original,count):
    spacing=SPAN/np.sqrt(count);axis=np.linspace(-SPAN/2,SPAN/2,257);x,y=np.meshgrid(axis,axis);xy=np.column_stack((x.ravel(),y.ravel()));h=reference_values(original,xy).reshape(257,257)
    curvature=np.abs(np.gradient(np.gradient(h,axis=0),axis=0))+np.abs(np.gradient(np.gradient(h,axis=1),axis=1))
    density=np.sqrt(gaussian_filter(curvature**2,max(1,spacing/125)));density=np.clip(.35+.65*density/max(density.mean(),1e-9),.25,3.);density/=density.mean()
    train_power=max(13,int(np.ceil(np.log2(count*2)))) if count>8192 else 13
    train=(qmc.Sobol(2,scramble=True,seed=199).random_base2(train_power)-.5)*SPAN;valid=(qmc.Sobol(2,scramble=True,seed=682).random_base2(12)-.5)*SPAN
    truth=reference_values(original,train);valid_truth=reference_values(original,valid);cover=xy[::16];best=None;history=[]
    for iteration in range(2):
        points=distribute(density,count,spacing).astype('<f4');tree=cKDTree(points)
        radius=max(4*spacing,float(tree.query(cover,k=min(12,count))[0][:,-1].max())*1.15)
        initial=reference_values(original,points);a=operator(points,train,radius);av=operator(points,valid,radius)
        base_rmse=float(np.sqrt(np.mean((av@initial-valid_truth)**2)))
        result=lsmr(a,truth-a@initial,damp=.15,atol=1e-5,btol=1e-5,maxiter=500)
        candidate=initial+result[0];score=float(np.sqrt(np.mean((av@candidate-valid_truth)**2)))
        adjusted=result[1] in (0,1,2,4,5) and score<base_rmse
        heights=candidate if adjusted else initial;score=score if adjusted else base_rmse
        history.append(dict(iteration=iteration+1,validationRmse=score,sampleHeightRmse=base_rmse,heightAdjustment=bool(adjusted),solverStatus=int(result[1]),solverIterations=int(result[2]),radiusMeters=radius))
        if best is None or score<best[0]:best=(score,points,heights.astype('<f4'),radius)
        err=(a@heights-truth);grid=np.zeros((257,257));mass=np.zeros_like(grid);ij=np.clip(((train/SPAN+.5)*256).astype(int),0,256)
        np.add.at(grid,(ij[:,1],ij[:,0]),err**2);np.add.at(mass,(ij[:,1],ij[:,0]),1)
        sigma=max(2,spacing/125);rms=np.sqrt(gaussian_filter(grid,sigma)/np.maximum(gaussian_filter(mass,sigma),1e-9));proposal=np.clip(density*np.sqrt((rms+1)/(rms.mean()+1)),.25,3.);density=.5*density+.5*proposal;density/=density.mean()
    return best,history

def refine(a):
    """保存XYを固定し、近傍の狭いMLSと高さ再適合を比較する。"""
    a.output.mkdir(parents=True,exist_ok=True);path=a.output/'mls.sqlite3'
    if path.exists():raise ValueError('既存データを残すため新しい出力先を指定してください')
    report=[]
    with sqlite3.connect(a.source_models/'mls.sqlite3') as models,sqlite3.connect(a.input) as source,sqlite3.connect(path) as db:
        db.execute('CREATE TABLE surfaces(region TEXT,budget INTEGER,info TEXT,points BLOB,PRIMARY KEY(region,budget)) WITHOUT ROWID')
        for region,count,raw,blob in models.execute('SELECT * FROM surfaces ORDER BY region,budget'):
            info=json.loads(raw)
            if count<8192:
                db.execute('INSERT INTO surfaces VALUES(?,?,?,?)',(region,count,raw,blob));report.append(info);continue
            start=time.monotonic();data=np.frombuffer(zlib.decompress(blob),'<f4')[1:].reshape(-1,3);points=data[:,:2];support=LocalSupport(points)
            original_blob=source.execute('SELECT coefficients FROM surfaces WHERE region=? AND n=256',(region,)).fetchone()[0];original=np.frombuffer(zlib.decompress(original_blob),'<f4').reshape(261,261)
            train=(qmc.Sobol(2,scramble=True,seed=199).random_base2(max(13,int(np.ceil(np.log2(count*2)))))-.5)*SPAN
            valid=(qmc.Sobol(2,scramble=True,seed=682).random_base2(12)-.5)*SPAN;truth=reference_values(original,train);vt=reference_values(original,valid)
            initial=reference_values(original,points);matrix=operator(points,train,support,curvature=1e-5);validation=operator(points,valid,support,curvature=1e-5)
            result=lsmr(matrix,truth-matrix@initial,damp=.15,atol=1e-5,btol=1e-5,maxiter=800);candidate=initial+result[0]
            accepted=result[1] in (0,1,2,4,5) and np.mean((validation@candidate-vt)**2)<np.mean((validation@initial-vt)**2)
            z=np.asarray(candidate if accepted else initial,'<f4');errors=validation@z-vt;radii=support(valid)
            peak=float(abs((operator(points,np.array([[0.,0.]]),support,curvature=1e-5)@z)[0]-reference_values(original,np.array([[0.,0.]]))[0]))
            packed=zlib.compress(np.array([1.25],'<f4').tobytes()+np.column_stack((points,z)).astype('<f4').tobytes(),6)
            info.update(kernel='MLS-quadratic-local-support-v1',supportMode='local-C2-v1',curvaturePenalty=1e-5,radiusHeaderName='neighborRadiusScale',radiusMeters=float(np.median(radii)),radiusMin=float(radii.min()),radiusMax=float(radii.max()),previousRmse=info['rmse'],rmse=float(np.sqrt(np.mean(errors**2))),maxError=float(abs(errors).max()),peakError=peak,storedBytes=len(packed),refitAccepted=bool(accepted),refitIterations=int(result[2]),seconds=time.monotonic()-start)
            db.execute('INSERT INTO surfaces VALUES(?,?,?,?)',(region,count,json.dumps(info),packed));db.commit();report.append(info);print(region,count,info['previousRmse'],info['rmse'],info['radiusMin'],info['radiusMeters'],info['radiusMax'],flush=True)
    (a.output/'report.json').write_text(json.dumps(report,indent=2),encoding='utf-8')


def field(points,heights,radius,n=256,curvature=1e-3):
    axis=np.linspace(-SPAN/2,SPAN/2,n+1);x,y=np.meshgrid(axis,axis);xy=np.column_stack((x.ravel(),y.ravel()));h=(operator(points,xy,radius,curvature=curvature)@heights).reshape(n+1,n+1)
    gy,gx=np.gradient(h,SPAN/n,edge_order=2);return np.stack((h,gx,gy,np.zeros_like(h)),axis=2).astype('<f4')

def generate(a):
    a.output.mkdir(parents=True,exist_ok=True);path=a.output/'mls.sqlite3'
    if path.exists() and not a.extend:raise ValueError('既存データを残すため新しい出力先か --extend を指定してください')
    report=[]
    with sqlite3.connect(a.input) as source,sqlite3.connect(path) as db:
        db.execute('CREATE TABLE IF NOT EXISTS surfaces(region TEXT,budget INTEGER,info TEXT,points BLOB,PRIMARY KEY(region,budget)) WITHOUT ROWID')
        for region,raw,blob in source.execute('SELECT region,info,coefficients FROM surfaces WHERE n=256'):
            original=np.frombuffer(zlib.decompress(blob),'<f4').reshape(261,261);ref_info=json.loads(raw)
            for count in COUNTS:
                if db.execute('SELECT 1 FROM surfaces WHERE region=? AND budget=?',(region,count)).fetchone():continue
                start=time.monotonic();(score,points,z,radius),history=optimize(original,count);radius=float(np.float32(radius));data=np.column_stack((points,z)).astype('<f4');packed=zlib.compress(np.array([radius],'<f4').tobytes()+data.tobytes(),6)
                validation=(qmc.Sobol(2,scramble=True,seed=682).random_base2(12)-.5)*SPAN;error=operator(points,validation,radius)@z-reference_values(original,validation);peak_error=float(abs((operator(points,np.array([[0.,0.]]),radius)@z)[0]-reference_values(original,np.array([[0.,0.]]))[0]));score=float(np.sqrt(np.mean(error**2)))
                info=dict(region=region,budget=count,controls=count,kernel='MLS-quadratic-Wendland-C2',mls=True,radiusMeters=radius,spacingMeters=SPAN/np.sqrt(count),storedBytes=len(packed),radiusHeaderBytes=4,rawBytes=data.nbytes+4,rmse=score,maxError=float(abs(error).max()),peakError=peak_error,originalBytes=len(blob),summit=ref_info['summit'],allocationHistory=history,seconds=time.monotonic()-start)
                db.execute('INSERT INTO surfaces VALUES(?,?,?,?)',(region,count,json.dumps(info),packed));db.commit();report.append(info);print(region,count,score,len(packed),info['seconds'],flush=True)
    with sqlite3.connect(path) as db:report=[json.loads(row[0]) for row in db.execute('SELECT info FROM surfaces ORDER BY region,budget')]
    (a.output/'report.json').write_text(json.dumps(report,indent=2),encoding='utf-8')

def serve(a):
    @functools.lru_cache(maxsize=128)
    def packet(region,count,pre_smooth,peak_mode,quality,quality_method):
        if quality!='fixed':
            if region!='fuji':raise ValueError('誤差指定の比較は現在富士山で利用できます')
            selected_output=a.downgrade_output if quality_method=='downgrade' else a.quality_output
            with sqlite3.connect(selected_output/'quality.sqlite3') as db:raw,blob=db.execute('SELECT info,points FROM levels WHERE region=? AND target_error=?',(region,float(quality))).fetchone()
            info=json.loads(raw);count=info['controls'];info['peakMode']='quality';info['qualityMethod']=quality_method;pre_smooth=False
            with sqlite3.connect(a.quality_output/'quality.sqlite3') as compare:
                other=json.loads(compare.execute('SELECT info FROM levels WHERE region=? AND target_error=?',(region,float(quality))).fetchone()[0])
            info.update(independentControls=other['controls'],independentBytes=other['storedBytes'],independentSeconds=other['seconds'])
        else:
            selected_output=a.peak_output if region=='fuji' and count==2048 and peak_mode=='peak' else a.local_output if region=='fuji' and count==2048 and peak_mode=='local' else a.output
            with sqlite3.connect(selected_output/'mls.sqlite3') as db:raw,blob=db.execute('SELECT info,points FROM surfaces WHERE region=? AND budget=?',(region,count)).fetchone()
            info=json.loads(raw);info['peakMode']=peak_mode if region=='fuji' and count==2048 else 'none'
        decoded=np.frombuffer(zlib.decompress(blob),'<f4');radius=float(decoded[0]);data=decoded[1:].reshape(-1,3)
        if info.get('supportMode')=='local-C2-v1':radius=LocalSupport(data[:,:2],scale=radius,pre_smooth=pre_smooth,floor_factor=info.get('supportFloorFactor',2),grid_n=info.get('supportGridN',128))
        else:info['radiusMeters']=radius
        curvature=info.get('curvaturePenalty',1e-3);info['preSmooth']=pre_smooth if callable(radius) else None
        field_n=512 if count>=8192 or info.get('peakMode') in ('local','peak','quality') else 256;info['fieldStepMeters']=SPAN/field_n
        quality_tag=f'-quality-{quality}' if quality!='fixed' else ''
        cache=selected_output/f'derived-{region}-{count}{quality_tag}{"-no-pre-smooth" if callable(radius) and not pre_smooth else ""}.npy'
        if cache.exists():grid=np.load(cache,allow_pickle=False)
        else:grid=field(data[:,:2],data[:,2],radius,n=field_n,curvature=curvature);np.save(cache,grid,allow_pickle=False)
        with sqlite3.connect(a.input) as db:refraw,refblob=db.execute('SELECT info,coefficients FROM surfaces WHERE region=? AND n=256',(region,)).fetchone()
        ref=json.loads(refraw)
        validation=(qmc.Sobol(2,scramble=True,seed=682).random_base2(12)-.5)*SPAN;actual=operator(data[:,:2],validation,radius,curvature=curvature)@data[:,2];truth=reference_values(np.frombuffer(zlib.decompress(refblob),'<f4').reshape(261,261),validation);error=actual-truth
        if callable(radius):
            radii=radius(validation);info.update(radiusMeters=float(np.median(radii)),radiusMin=float(radii.min()),radiusMax=float(radii.max()))
        info.update(rmse=float(np.sqrt(np.mean(error**2))),maxError=float(abs(error).max()),peakError=float(abs((operator(data[:,:2],np.array([[0.,0.]]),radius,curvature=curvature)@data[:,2])[0]-reference_values(np.frombuffer(zlib.decompress(refblob),'<f4').reshape(261,261),np.array([[0.,0.]]))[0])))
        if info.get('peakMode') in ('base','local','peak'):
            distance=np.linalg.norm(data[:,:2],axis=1);info.update(within500=int((distance<=500).sum()),within1000=int((distance<=1000).sum()))
            from scipy.stats import norm
            focus=np.clip(norm.ppf(qmc.Sobol(2,scramble=True,seed=881).random_base2(12)),-5,5)*1000
            focus_error=operator(data[:,:2],focus,radius,curvature=curvature)@data[:,2]-reference_values(np.frombuffer(zlib.decompress(refblob),'<f4').reshape(261,261),focus)
            info['peakAreaRmse']=float(np.sqrt(np.mean(focus_error**2)))
        displayed=map_coordinates(grid[:,:,0],((validation/SPAN+.5)*field_n).T[[1,0]],order=1,prefilter=False);info['displayRmse']=float(np.sqrt(np.mean((displayed-actual)**2)));info['displayMax']=float(abs(displayed-actual).max())
        enc=lambda v:base64.b64encode(v.tobytes()).decode();features=np.column_stack((data[:,:2],np.zeros(count),data[:,2])).astype('<f4')
        model=dict(info=info,span=SPAN,base=[0,0,0],kernelMode=4,fieldN=field_n,fieldGrid=enc(grid),features=enc(features),triangles=enc(np.zeros(16,'<f4')),lookup=enc(np.zeros((1,2),'<u4')),indices=enc(np.zeros(1,'<u4')),binN=1,maxBin=0,reference=dict(info=ref,size=261,halo=2,span=SPAN,summit=ref['summit'],coefficients=base64.b64encode(zlib.decompress(refblob)).decode()))
        return gzip.compress(json.dumps(model).encode(),1)
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            try:
                u=urlparse(self.path);q=parse_qs(u.query);encoding=None
                if u.path=='/api/adaptive':body=packet(q.get('region',['yari'])[0],int(q.get('budget',['512'])[0]) if q.get('quality',['fixed'])[0]=='fixed' else 0,q.get('preSmooth',['0'])[0]!='0',q.get('peakMode',['peak'])[0],q.get('quality',['fixed'])[0],q.get('qualityMethod',['independent'])[0]);mime='application/json';encoding='gzip'
                elif u.path=='/':
                    with urllib.request.urlopen('http://127.0.0.1:61473/') as r:s=r.read().decode()
                    s=s.replace('自由配置の曲面</option>','自由配置のMLS曲面</option>')
                    start=s.index('<select id="adaptiveLayout">');end=s.index('</select>',start);s=s[:start]+'<select id="adaptiveLayout"><option value="mls" selected>移動最小二乗（MLS）</option>'+s[end:]
                    start=s.index('<select id="adaptiveBudget">');end=s.index('</select>',start);s=s[:start]+'<select id="adaptiveBudget">'+'<option value="auto" disabled>品質から自動決定</option>'+''.join(f'<option value="{n}"'+(' selected' if n==512 else '')+f'>{n:,}点</option>' for n in COUNTS)+s[end:]
                    s=s.replace('<button id="ridgeFocus">','<label>品質の生成 <select id="mlsQualityMethod"><option value="independent" selected>品質ごとに独立生成</option><option value="downgrade">高品位から点を削減</option></select></label><button id="ridgeFocus">')
                    s=s.replace('<button id="ridgeFocus">','<label>富士山の品質 <select id="mlsQuality"><option value="fixed" selected>固定点数（比較）</option><option value="200">E200・最大200m</option><option value="100">E100・最大100m</option><option value="50">E50・最大50m</option><option value="25">E25・最大25m</option></select></label><button id="ridgeFocus">')
                    s=s.replace('<button id="ridgeFocus">','<label>富士山2048点の配分 <select id="mlsPeakMode"><option value="base">従来</option><option value="local">配置そのまま・局所近似</option><option value="peak" selected>山頂重点・局所近似</option></select></label><button id="ridgeFocus">')
                    s=s.replace('<button id="ridgeFocus">','<label><input type="checkbox" id="mlsPreSmooth">半径分布を事前に平滑化</label><button id="ridgeFocus">')
                    s=s.replace('<script src="/lpf_curve_compare.js">','<script>window.prunedSplineEnabled=true;window.mlsComparison=true;</script><script src="/lpf_curve_compare.js">')
                    s=s.replace('id="adaptiveFairing"','id="adaptiveFairing" disabled').replace('value="40" selected','value="40"').replace('value="0">なし（従来）','value="0" selected>なし（MLS）').replace('id="splineControlPoints" checked','id="splineControlPoints"')
                    start=s.index('<div id="note">');end=s.index('</div>',start);s=s[:start]+'<div id="note">左：元のBスプライン。右：任意のXYに置いた高さ点から移動最小二乗で復元した曲面。格子や三角形の接続は保存しません。周囲の点へ二次曲面を当てはめます。8,192点以上は半径分布の事前平滑化を切り替えられます。初期状態はOFF。標高への事前LPFはどちらも掛けていません。富士山2048点は従来・配置そのままの局所近似・山頂重点配分を比較できます。総点数は同じで、山頂の周囲へ滑らかに密度を高めます。MLSの重み付き近似は残します。曲率と再現誤差から2次元で点を再配分し、高さも別の評価点で改善した場合に調整します。表示は125m以下の間隔で一時評価した高さと傾きをGPUでポリゴン化します。この描画用格子は配布データではありません。低点数では山頂も丸まります。品質指定は点数を自動決定します。最大誤差は125mの確認格子と別の評価点で測り、連続領域全体の保証ではありません。山頂周辺は半分の誤差を目標にします。Tスプラインは使用していません。</div>'+s[end+6:];body=s.encode();mime='text/html; charset=utf-8'
                elif u.path=='/adaptive_surface.js':body=Path('tools/terrain/adaptive_surface.js').read_bytes();mime='text/javascript'
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
    p=argparse.ArgumentParser();p.add_argument('action',choices=['generate','serve','refine']);p.add_argument('--input',type=Path,default=Path('build/ridge-spline-prototype-v4/spline.sqlite3'));p.add_argument('--output',type=Path,default=Path('build/free-mls-v3'));p.add_argument('--port',type=int,default=61483);p.add_argument('--extend',action='store_true');p.add_argument('--source-models',type=Path,default=Path('build/free-mls-v2'));p.add_argument('--peak-output',type=Path,default=Path('build/free-mls-fuji-peak20-v1'));p.add_argument('--local-output',type=Path,default=Path('build/free-mls-fuji-local-v1'));p.add_argument('--quality-output',type=Path,default=Path('build/mls-error-quality-v7'));p.add_argument('--downgrade-output',type=Path,default=Path('build/mls-downgrade-v2'));a=p.parse_args();dict(generate=generate,serve=serve,refine=refine)[a.action](a)
