"""共通XY・レベル別高さのLPF／Clough–Tocher曲面を比較する地域試作。"""
import argparse
import functools
import gzip
import json
import lzma
import math
import sqlite3
import struct
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

import numpy as np
from scipy.fft import irfft2, next_fast_len, rfft2
from scipy.interpolate import CloughTocher2DInterpolator
from scipy.ndimage import fourier_gaussian, map_coordinates
from scipy.spatial import Delaunay, cKDTree

WORLD = 40075016.68557849
UNIT = WORLD / 268435456
SOURCE_STEP = WORLD / 4194304
EXP = np.array([[3,0,0],[0,3,0],[0,0,3],[2,1,0],[1,2,0],[2,0,1],[1,0,2],[0,2,1],[0,1,2],[1,1,1]])
FACTOR = np.array([1,1,1,3,3,3,3,3,3,6])
NODES = EXP / 3

def basis(bary):
    return np.prod(np.power(bary[..., None, :], EXP), axis=-1) * FACTOR

INVERSE = np.linalg.inv(basis(NODES))

def relaxed_heights(xy, heights, maximum=12.5):
    """XYを固定し、局所2次近似の高さを許容範囲内だけ採用する。"""
    if len(xy)<12:
        return heights.copy()
    lengths,ids=cKDTree(xy).query(xy,k=min(24,len(xy)))
    radius=np.maximum(lengths[:,-1],1.)
    q=(xy[ids]-xy[:,None,:])/radius[:,None,None]
    x,y=q[:,:,0],q[:,:,1]
    design=np.stack([np.ones_like(x),x,y,x*x,x*y,y*y],axis=-1)
    weights=np.exp(-4*(x*x+y*y))
    a=np.einsum('nki,nk,nkj->nij',design,weights,design)
    a+=np.eye(6)[None,:,:]*1e-10
    current=heights.copy()
    boundary=np.unique(Delaunay(xy).convex_hull)
    for _ in range(2):
        b=np.einsum('nki,nk,nk->ni',design,weights,current[ids])
        fit=np.linalg.solve(a,b[...,None])[:,0,0]
        current=heights+np.clip(fit-heights,-maximum,maximum)
        current[boundary]=heights[boundary]
    return current

class Source:
    def __init__(self, path):
        self.db = sqlite3.connect(Path(path).resolve().as_uri() + '?mode=ro', uri=True)

    @functools.lru_cache(maxsize=128)
    def tile(self, x, y):
        row = self.db.execute('SELECT data FROM tiles WHERE z=14 AND x=? AND y=?', (x,y)).fetchone()
        if row is None:
            return np.zeros((256,256), np.float32)
        b = row[0]
        magic, rows, cols, step, base, flags, codec = struct.unpack_from('<4sHHfiBB', b)
        assert magic == b'YMD1' and (rows,cols) == (256,256)
        raw = (gzip.decompress if codec == 0 else lzma.decompress)(b[18:])
        width = 4 if flags & 1 else 2
        d = np.frombuffer(raw, '<i4' if width == 4 else '<i2', rows*cols).reshape(rows,cols)
        grid = ((d.cumsum(0, dtype=np.int64).cumsum(1) + base) * step).astype(np.float32)
        if flags & 2:
            mask = np.unpackbits(np.frombuffer(raw[rows*cols*width:], np.uint8), bitorder='little')[:rows*cols]
            grid.ravel()[mask != 0] = 0
        return grid

    def sample(self, x, y):
        x,y = np.broadcast_arrays(np.asarray(x),np.asarray(y))
        px,py = x.ravel()/SOURCE_STEP-0.5,y.ravel()/SOURCE_STEP-0.5
        ix,iy = np.floor(px).astype(np.int64),np.floor(py).astype(np.int64)
        out = np.zeros(len(px))
        for dx,dy in [(0,0),(1,0),(0,1),(1,1)]:
            xx,yy=ix+dx,iy+dy
            keys=(yy//256)*16384+xx//256
            order=np.argsort(keys,kind='stable')
            starts=np.r_[0,np.flatnonzero(np.diff(keys[order]))+1,len(order)]
            weight=(px-ix if dx else 1-(px-ix))*(py-iy if dy else 1-(py-iy))
            for a,b in zip(starts[:-1],starts[1:]):
                ids=order[a:b]; key=int(keys[ids[0]])
                out[ids]+=self.tile(key%16384,key//16384)[yy[ids]%256,xx[ids]%256]*weight[ids]
        return out.reshape(x.shape)

def patches(tri, heights, gradient=None):
    curve=CloughTocher2DInterpolator(tri,heights)
    # 明示勾配は旧方式の比較用。通常は高さ群に適合する共通勾配を推定する。
    if gradient is not None:
        curve.grad[:,0,:]=gradient
    count=len(tri.simplices)
    coefficients=np.empty((count,3,10),np.float32)
    worst=0.
    for start in range(0,count,2048):
        ids=tri.simplices[start:start+2048]
        p=tri.points[ids]; center=p.mean(1)
        corners=np.stack([np.stack([p[:,k],p[:,(k+1)%3],center],1) for k in range(3)],1)
        query=np.einsum('na,fpac->fpnc',NODES,corners)
        values=curve(query.reshape(-1,2)).reshape(len(ids),3,10)
        coef=values@INVERSE.T
        assert np.isfinite(coef).all()
        coefficients[start:start+len(ids)]=coef
        bary=np.array([[0.21,0.34,0.45],[0.61,0.13,0.26]])
        check=np.einsum('na,fpac->fpnc',bary,corners)
        expected=curve(check.reshape(-1,2)).reshape(len(ids),3,2)
        actual=np.einsum('nk,fpk->fpn',basis(bary),coef.astype(np.float32))
        worst=max(worst,float(np.max(np.abs(actual-expected))))
    assert worst<0.02, worst
    return coefficients,curve,worst

def linear_values(tri,h,query):
    index=tri.find_simplex(query)
    assert np.all(index>=0)
    a=np.einsum('nij,nj->ni',tri.transform[index,:2],query-tri.transform[index,2])
    b=np.column_stack((a,1-a.sum(1)))
    return np.sum(h[tri.simplices[index]]*b,axis=1)

def generate(args):
    args.output.mkdir(parents=True,exist_ok=True)
    target=args.output/'comparison.sqlite3'
    if target.exists() and not args.resume: raise RuntimeError('既存の比較データは上書きしません。再開は--resumeを指定してください')
    inp=sqlite3.connect(args.input.resolve().as_uri()+'?mode=ro',uri=True)
    out=sqlite3.connect(target)
    out.executescript('CREATE TABLE IF NOT EXISTS regions(name TEXT PRIMARY KEY,info TEXT);CREATE TABLE IF NOT EXISTS levels(region TEXT,z INTEGER,info TEXT,xy_new BLOB,height_original BLOB,height_delta BLOB,triangles BLOB,coefficients BLOB,PRIMARY KEY(region,z)) WITHOUT ROWID;')
    source=Source(args.source)
    report=[]
    started=time.time()
    def progress(**v):
        (args.output/'progress.json').write_text(json.dumps(dict(elapsed=time.time()-started,**v),ensure_ascii=False),encoding='utf-8')
    for name,cx,cy,_ in inp.execute('SELECT * FROM regions ORDER BY name'):
        existing=out.execute('SELECT info FROM levels WHERE region=? ORDER BY z',(name,)).fetchall()
        if len(existing)==9:
            report.extend({k:v for k,v in json.loads(row[0]).items() if k!='profile'} for row in existing)
            continue
        center=np.array([cx,cy])*UNIT
        lat=math.degrees(math.atan(math.sinh(math.pi*(1-2*center[1]/WORLD))))
        cosine=math.cos(math.radians(lat))
        half=16000.
        grid_step=31.25
        sigma_max=2000./cosine
        n=next_fast_len(math.ceil((2*half+12*sigma_max)/grid_step)+1)
        lower=center-(n-1)*grid_step/2
        raw=np.empty((n,n),np.float32)
        xs=lower[0]+np.arange(n)*grid_step
        for row in range(0,n,32):
            progress(region=name,stage='read-source',row=row,total=n)
            ys=lower[1]+np.arange(row,min(row+32,n))*grid_step
            raw[row:row+len(ys)]=source.sample(xs[None,:],ys[:,None])
        spectrum=rfft2(raw,workers=4)
        previous=[];previous_height=np.array([])
        region_info=dict(name=name,center=center.tolist(),half=half,gridStep=grid_step,latitude=lat)
        saved=out.execute('SELECT info FROM regions WHERE name=?',(name,)).fetchone()
        if saved: assert json.loads(saved[0])==region_info, '保存済み地域の設定が異なります'
        else: out.execute('INSERT INTO regions VALUES(?,?)',(name,json.dumps(region_info)))
        for z in range(6,15):
            saved=out.execute('SELECT info FROM levels WHERE region=? AND z=?',(name,z)).fetchone()
            if saved:
                saved_info,saved_xy,saved_h,*_=load_level(out,name,z)
                assert bool(saved_info.get('curveFitGridError') is not None)==bool(args.fit)
                assert saved_info.get('gradientSource','dem')==args.gradient_source
                previous=[tuple(map(int,p)) for p in saved_xy];previous_height=saved_h
                report.append({k:v for k,v in saved_info.items() if k!='profile'})
                print(name,z,'保存済みを引き継ぎ',len(previous),flush=True)
                continue
            progress(region=name,z=z,stage='filter-and-fit')
            sigma=0. if z==14 else 31.25*2**(12-z)
            filtered=raw if sigma==0 else irfft2(fourier_gaussian(spectrum,sigma=sigma/cosine/grid_step,n=n,axis=-1),s=raw.shape,workers=4).astype(np.float32)
            gy,gx=np.gradient(filtered,grid_step)
            rows=inp.execute('SELECT x,y,h FROM points WHERE region=? AND z=? ORDER BY x,y',(name,z)).fetchall()
            old=set(previous)
            candidate={(int(x),int(y)):h for x,y,h in rows if abs(x*UNIT-center[0])<=half and abs(y*UNIT-center[1])<=half}
            corner_half=round(half/UNIT/64)*64
            for dx,dy in [(-1,-1),(1,-1),(1,1),(-1,1)]:
                k=(round(cx)+dx*corner_half,round(cy)+dy*corner_half)
                candidate[k]=float(source.sample(k[0]*UNIT,k[1]*UNIT))
            order=previous+sorted(set(candidate)-old)
            assert old<=set(order) and len(set(order))==len(order)
            inherited_count=len(order)
            fit_added=0
            fit_tolerance=max(12.5,12.5*2**(14-z)*0.125)
            if args.fit:
                # 全地域で同じ格子。曲面そのものの誤差で追加し、親のXYは動かさない。
                axis=np.linspace(-half*0.995,half*0.995,129)
                qx,qy=np.meshgrid(axis,axis)
                qxy=np.round((np.column_stack((qx.ravel(),qy.ravel()))+center)/UNIT/64).astype(np.int64)*64
                query_xy=qxy*UNIT-center
                qc=((qxy*UNIT-lower)/grid_step)[:,::-1].T
                target_h=source.sample(qxy[:,0]*UNIT,qxy[:,1]*UNIT) if z==14 else map_coordinates(filtered,qc,order=3,mode='nearest')
                for iteration in range(2049):
                    fit_xy=np.array(order)*UNIT-center
                    fit_coord=((np.array(order)*UNIT-lower)/grid_step)[:,::-1].T
                    fit_h=source.sample(np.array(order)[:,0]*UNIT,np.array(order)[:,1]*UNIT) if z==14 else map_coordinates(filtered,fit_coord,order=3,mode='nearest')
                    fit_g=np.column_stack([map_coordinates(g,fit_coord,order=1,mode='nearest') for g in [gx,gy]])
                    fit_tri=Delaunay(fit_xy)
                    trial=CloughTocher2DInterpolator(fit_tri,fit_h)
                    if args.gradient_source=='dem':trial.grad[:,0,:]=fit_g
                    errors=np.abs(trial(query_xy)-target_h)
                    fit_error_grid=float(errors.max())
                    if fit_error_grid<=fit_tolerance:break
                    if iteration==2048:raise RuntimeError(f'{name} z{z}: 曲面の適合が収束しません {fit_error_grid}')
                    # 離れた候補をまとめ、追加後に全候補を再評価する。
                    selected=set(order)
                    batch=[]
                    for idx in np.argsort(errors)[::-1]:
                        if errors[idx]<=fit_tolerance:break
                        k=tuple(map(int,qxy[idx]))
                        if k not in selected and all(np.linalg.norm((qxy[idx]-qxy[j])*UNIT)>=max(250.,sigma/cosine/2) for j in batch):
                            batch.append(int(idx))
                            if len(batch)>=32:break
                    if not batch:raise RuntimeError('追加可能な評価点がありません')
                    for idx in batch:
                        k=tuple(map(int,qxy[idx]));order.append(k);candidate[k]=float(source.sample(k[0]*UNIT,k[1]*UNIT));fit_added+=1
                    if iteration%16==0:progress(region=name,z=z,stage='fit-curved-surface',added=fit_added,error=fit_error_grid,tolerance=fit_tolerance)
            xy=np.array(order,np.int64); local=xy*UNIT-center
            tri=Delaunay(local)
            original=np.array([candidate.get(p,np.nan) for p in order])
            missing=~np.isfinite(original)
            if missing.any(): original[missing]=source.sample(xy[missing,0]*UNIT,xy[missing,1]*UNIT)
            coord=((xy*UNIT-lower)/grid_step)[:,::-1].T
            h=original.copy() if z==14 else map_coordinates(filtered,coord,order=3,mode='nearest')
            gradient=np.column_stack([map_coordinates(g,coord,order=1,mode='nearest') for g in [gx,gy]])
            coef,curve,fit_error=patches(tri,h,gradient if args.gradient_source=='dem' else None)
            delta=h.copy();delta[:len(previous)]-=previous_height
            # 誤差なく親XYを復元し、高さは独立した差分で復元する。
            stored=delta.astype('<f4');restored=stored.astype(float);restored[:len(previous)]+=previous_height
            assert np.max(np.abs(restored-h))<0.001
            h=restored
            query=np.column_stack([np.linspace(-half*0.98,half*0.98,513),np.zeros(513)])
            profile=dict(x=query[:,0].tolist(),original=linear_values(tri,original,query).tolist(),filtered=linear_values(tri,h,query).tolist(),curve=curve(query).tolist(),reference=source.sample(center[0]+query[:,0],center[1]+query[:,1]).tolist())
            faces=tri.simplices.astype('<u4')
            sizes=dict(newXY=(len(order)-len(previous))*8,original=original.size*4,heightDelta=stored.nbytes,triangles=faces.nbytes,curveCoefficients=coef.nbytes)
            info=dict(region=name,z=z,vertices=len(order),faces=len(faces),parentCount=len(previous),sigmaMeters=sigma,coefficientFitError=fit_error,sizes=sizes,profile=profile,originalMaximum=float(original.max()),filteredMaximum=float(h.max()),fitAdded=fit_added,initialVertices=inherited_count,curveFitGridMeters=half*1.99/128,curveFitTolerance=fit_tolerance,curveFitGridError=fit_error_grid if args.fit else None,gradientSource=args.gradient_source)
            out.execute('INSERT INTO levels VALUES(?,?,?,?,?,?,?,?)',(name,z,json.dumps(info),xy[len(previous):].astype('<i4').tobytes(),original.astype('<f4').tobytes(),stored.tobytes(),faces.tobytes(),coef.tobytes()))
            out.commit();report.append({k:v for k,v in info.items() if k!='profile'})
            previous=order;previous_height=restored
            print(name,z,len(order),len(faces),'sigma',sigma,flush=True)
        del spectrum,raw,filtered,gx,gy
    out.close()
    (args.output/'report.json').write_text(json.dumps(dict(levels=report,sqliteBytes=target.stat().st_size),ensure_ascii=False,indent=2),encoding='utf-8')
    progress(stage='complete',complete=True)

def load_level(db,region,z):
    points=np.empty((0,2),np.float64);heights=np.empty(0,np.float64)
    for level in range(6,z+1):
        row=db.execute('SELECT info,xy_new,height_original,height_delta,triangles,coefficients FROM levels WHERE region=? AND z=?',(region,level)).fetchone()
        if row is None: raise ValueError('生成されていない地域・レベルです')
        info=json.loads(row[0]);points=np.vstack((points,np.frombuffer(row[1],'<i4').reshape(-1,2)))
        delta=np.frombuffer(row[3],'<f4').astype(float);delta[:len(heights)]+=heights;heights=delta
    return info,points,heights,np.frombuffer(row[2],'<f4'),np.frombuffer(row[4],'<u4').reshape(-1,3),np.frombuffer(row[5],'<f4').reshape(-1,3,10)

def smooth(args):
    args.output.mkdir(parents=True,exist_ok=True)
    target=args.output/'comparison.sqlite3'
    if target.exists():raise RuntimeError('平滑化の出力は新しいディレクトリを指定してください')
    source=sqlite3.connect(args.base.resolve().as_uri()+'?mode=ro',uri=True)
    out=sqlite3.connect(target);source.backup(out)
    report=[]
    for name,meta in source.execute('SELECT name,info FROM regions ORDER BY name'):
        center=np.array(json.loads(meta)['center']);previous=np.empty(0)
        for z in range(6,15):
            (args.output/'progress.json').write_text(json.dumps(dict(region=name,z=z,stage='smooth-curves')),encoding='utf-8')
            info,xy,h,_,faces,_=load_level(source,name,z)
            local=xy*UNIT-center
            new_h=relaxed_heights(local,h)
            delta=new_h.copy();delta[:len(previous)]-=previous
            stored=delta.astype('<f4');restored=stored.astype(float);restored[:len(previous)]+=previous
            assert np.max(abs(restored-new_h))<0.001
            tri=Delaunay(local);assert np.array_equal(tri.simplices,faces)
            coef,curve,error=patches(tri,restored)
            query=np.column_stack([np.array(info['profile']['x']),np.zeros(513)])
            info['profile']['filtered']=linear_values(tri,restored,query).tolist()
            info['profile']['curve']=curve(query).tolist()
            info['curveFitGridErrorBeforeRelaxation']=info.get('curveFitGridError')
            info['curveFitGridError']=None
            info['gradientSource']='relaxed-height-fit'
            info['heightRelaxationMax']=12.5
            info['heightRelaxationStatistics']=np.quantile(abs(restored-h),[0.5,0.95,1]).tolist()
            info['coefficientFitError']=error;info['filteredMaximum']=float(restored.max())
            out.execute('UPDATE levels SET info=?,height_delta=?,coefficients=? WHERE region=? AND z=?',(json.dumps(info),stored.tobytes(),coef.tobytes(),name,z));out.commit()
            previous=restored
            report.append({k:v for k,v in info.items() if k!='profile'})
            print(name,z,'平滑化',info['heightRelaxationStatistics'],flush=True)
    out.close()
    (args.output/'report.json').write_text(json.dumps(dict(levels=report,sqliteBytes=target.stat().st_size),ensure_ascii=False,indent=2),encoding='utf-8')
    (args.output/'progress.json').write_text(json.dumps(dict(stage='complete',complete=True)),encoding='utf-8')

def serve(args):
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            try:
                url=urlparse(self.path);q=parse_qs(url.query)
                if url.path=='/api/data':
                    region=q.get('region',['fuji'])[0];z=int(q.get('z',['6'])[0])
                    with sqlite3.connect((args.output/'comparison.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as db:
                        info,xy,h,original,faces,coef=load_level(db,region,z)
                        meta=json.loads(db.execute('SELECT info FROM regions WHERE name=?',(region,)).fetchone()[0])
                    xy=xy*UNIT-np.array(meta['center']);nv=len(xy)
                    vertices=np.column_stack((xy,original,h)).astype('<f4')
                    packet=struct.pack('<4sIII',b'LPC1',nv,len(faces),coef.size)+vertices.tobytes()+faces.astype('<u4').tobytes()+coef.tobytes()
                    body=gzip.compress(packet,compresslevel=1);mime='application/octet-stream';encoding='gzip'
                elif url.path=='/api/info':
                    with sqlite3.connect((args.output/'comparison.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as db:
                        levels=[json.loads(r[0]) for r in db.execute('SELECT info FROM levels ORDER BY region,z')]
                    body=json.dumps(dict(levels=levels),ensure_ascii=False).encode();mime='application/json; charset=utf-8';encoding=None
                elif url.path in ['/','/lpf_curve_compare.js','/lpf_curve_raycast.js']:
                    p=Path(__file__).with_suffix('.html') if url.path=='/' else Path(__file__).with_name(url.path[1:])
                    body=p.read_bytes();mime='text/html; charset=utf-8' if url.path=='/' else 'text/javascript; charset=utf-8';encoding=None
                else: self.send_error(404);return
                self.send_response(200);self.send_header('Content-Type',mime);self.send_header('Content-Length',str(len(body)))
                self.send_header('Cache-Control','no-cache')
                if encoding:self.send_header('Content-Encoding',encoding)
                self.end_headers();self.wfile.write(body)
            except (ValueError,RuntimeError) as e:self.send_error(400,str(e))
        def log_message(self,*args):pass
    print(f'http://127.0.0.1:{args.port}/',flush=True)
    ThreadingHTTPServer(('127.0.0.1',args.port),Handler).serve_forever()

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('action',choices=['generate','serve','smooth']);p.add_argument('--input',type=Path,default=Path('build/lpf-curve-prototype/input.sqlite3'));p.add_argument('--source',type=Path,default=Path('build/terrain-lod.sqlite3'));p.add_argument('--output',type=Path,default=Path('build/lpf-curve-prototype'));p.add_argument('--port',type=int,default=61464);p.add_argument('--fit',action='store_true');p.add_argument('--resume',action='store_true');p.add_argument('--gradient-source',choices=['estimated','dem'],default='estimated');p.add_argument('--base',type=Path,default=Path('build/lpf-curve-fit-prototype/comparison.sqlite3'))
    a=p.parse_args();dict(generate=generate,serve=serve,smooth=smooth)[a.action](a)
