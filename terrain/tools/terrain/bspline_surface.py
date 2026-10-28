"""三角形を保存しない、疎な階層3次Bスプラインの地域比較。"""
import argparse
import base64
import gzip
import json
import sqlite3
import time
import urllib.request
import zlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

import numpy as np
from scipy.ndimage import map_coordinates, spline_filter, maximum_filter

from lpf_curve_compare import Source

HALO = 16
SPAN = 32000.
SIZES = [8, 16, 32, 64, 128, 256, 512]


def evaluate(layers, x, y):
    x, y = np.broadcast_arrays(x, y)
    out = np.zeros(x.shape)
    for n, c in layers:
        coordinates = np.array([(y.ravel()+SPAN/2)*n/SPAN+HALO,
                                (x.ravel()+SPAN/2)*n/SPAN+HALO])
        out += map_coordinates(c, coordinates, order=3, prefilter=False,
                               mode='nearest').reshape(x.shape)
    return out


def flatten(layers):
    """同じ曲面を細かい一様基底へ変換。GPU用でありDEMではない。"""
    n = layers[-1][0]
    axis = (np.arange(n+1+2*HALO)-HALO)*SPAN/n-SPAN/2
    x, y = np.meshgrid(axis, axis)
    return spline_filter(evaluate(layers, x, y), order=3, mode='mirror').astype('<f4')


def generate(args):
    args.output.mkdir(parents=True, exist_ok=True)
    target = args.output/'spline.sqlite3'
    if target.exists():
        raise RuntimeError('既存出力を保持するため新しい出力先を指定してください')
    source = Source(args.source)
    meta_db = sqlite3.connect(args.base.resolve().as_uri()+'?mode=ro', uri=True)
    out = sqlite3.connect(target)
    out.executescript('CREATE TABLE regions(name TEXT PRIMARY KEY, info TEXT);'
                     'CREATE TABLE layers(region TEXT,n INTEGER,info TEXT,indices BLOB,coefficients BLOB,PRIMARY KEY(region,n)) WITHOUT ROWID;')
    reports = []
    rng = np.random.default_rng(734)
    for name, raw in meta_db.execute('SELECT name,info FROM regions ORDER BY name'):
        meta = json.loads(raw)
        center = np.array(meta['center'])
        out.execute('INSERT INTO regions VALUES(?,?)', (name, raw))
        layers = []
        axis = np.linspace(-SPAN/2, SPAN/2, 513)
        qx, qy = np.meshgrid(axis, axis)
        reference = source.sample(qx+center[0], qy+center[1])
        rx = rng.uniform(-SPAN/2, SPAN/2, 8192)
        ry = rng.uniform(-SPAN/2, SPAN/2, 8192)
        random_reference = source.sample(rx+center[0], ry+center[1])
        cumulative = 0
        for n in SIZES:
            start = time.monotonic()
            step = SPAN/n
            nodes = (np.arange(n+1+2*HALO)-HALO)*step-SPAN/2
            x, y = np.meshgrid(nodes, nodes)
            residual = source.sample(x+center[0], y+center[1])-evaluate(layers, x, y)
            c = spline_filter(residual, order=3, mode='mirror')
            # 大きな残差の近傍だけ係数を保持する。基底はC2でゼロへ接続する。
            cutoff = max(.5, 1000/n)
            if layers:
                active = maximum_filter(np.abs(residual), size=5, mode='nearest') > cutoff
                c[~active] = 0
            # 領域に基底の台が届かない係数は保存しない。
            c[:HALO-1,:]=0;c[:, :HALO-1]=0
            c[HALO+n+2:,:]=0;c[:,HALO+n+2:]=0
            c = c.astype('<f4')
            layers.append((n, c))
            indices = np.flatnonzero(c).astype('<u4')
            values = c.ravel()[indices].astype('<f4')
            packed_i = zlib.compress(indices.tobytes(), 6)
            packed_c = zlib.compress(values.tobytes(), 6)
            cumulative += len(packed_i)+len(packed_c)
            dense = flatten(layers)
            error = evaluate(layers, qx, qy)-reference
            random_error = evaluate(layers, rx, ry)-random_reference
            flat_error = float(np.max(np.abs(evaluate([(n,dense)],rx,ry)-evaluate(layers,rx,ry))))
            if flat_error > .02:
                raise RuntimeError(f'基底変換誤差が大きすぎます: {flat_error}')
            info = dict(region=name,n=n,stepMeters=step,coefficientCount=len(values),
                        cumulativeCoefficients=int(sum(np.count_nonzero(v) for _,v in layers)),
                        storedBytes=len(packed_i)+len(packed_c),cumulativeBytes=cumulative,
                        denseGpuBytes=dense.nbytes,rmse=float(np.sqrt(np.mean(error**2))),
                        maxError=float(np.max(np.abs(error))),randomMaxError=float(np.max(np.abs(random_error))),
                        evaluationGridMeters=62.5,randomSamples=len(rx),flattenError=flat_error,
                        cutoffMeters=cutoff,seconds=time.monotonic()-start)
            out.execute('INSERT INTO layers VALUES(?,?,?,?,?)',(name,n,json.dumps(info),packed_i,packed_c))
            out.commit()
            reports.append(info)
            print(json.dumps(info),flush=True)
            (args.output/'progress.json').write_text(json.dumps(info),encoding='utf-8')
    out.close()
    (args.output/'report.json').write_text(json.dumps(dict(levels=reports,sqliteBytes=target.stat().st_size),indent=2),encoding='utf-8')
    (args.output/'progress.json').write_text('{"complete":true}',encoding='utf-8')


def load(db, region, n):
    layers = []
    info = None
    for size, raw, ib, cb in db.execute('SELECT n,info,indices,coefficients FROM layers WHERE region=? AND n<=? ORDER BY n',(region,n)):
        indices = np.frombuffer(zlib.decompress(ib),'<u4')
        values = np.frombuffer(zlib.decompress(cb),'<f4')
        c = np.zeros((size+1+2*HALO)**2,np.float32)
        c[indices] = values
        layers.append((size,c.reshape(size+1+2*HALO,-1)))
        info = json.loads(raw)
    if info is None or info['n'] != n:
        raise ValueError('未生成の曲面です')
    return info,layers


def serve(args):
    import functools
    @functools.lru_cache(maxsize=24)
    def packet(region,n):
        with sqlite3.connect((args.output/'spline.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as db:
            info,layers=load(db,region,n)
        dense=flatten(layers)
        step=SPAN/n
        relevant=dense[HALO-2:HALO+n+3,HALO-2:HALO+n+3]
        # Bスプラインの微分は隣接係数差の凸結合。各軸の厳密な上界。
        bounds=[float(np.max(np.abs(np.diff(relevant,axis=axis)))/step) for axis in [1,0]]
        result=dict(info=info,size=dense.shape[0],halo=HALO,span=SPAN,
                    heightBounds=[float(relevant.min())-.1,float(relevant.max())+.1],gradientBounds=bounds,
                    coefficients=base64.b64encode(dense.tobytes()).decode())
        return gzip.compress(json.dumps(result).encode(),1)

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            try:
                url=urlparse(self.path);q=parse_qs(url.query)
                encoding=None
                if url.path=='/api/spline':
                    body=packet(q.get('region',['fuji'])[0],int(q.get('n',['128'])[0]));mime='application/json';encoding='gzip'
                elif url.path in ['/api/info','/api/data']:
                    with urllib.request.urlopen('http://127.0.0.1:61465'+self.path,timeout=120) as response:
                        body=response.read();mime=response.headers['Content-Type'];encoding=response.headers.get('Content-Encoding')
                elif url.path=='/':
                    html=Path(__file__).with_name('lpf_curve_compare.html').read_text(encoding='utf-8')
                    html=html.replace('LPF＋曲面／共通XY・レベル別高さの試作','階層Bスプライン／三角形を保存しない曲面')
                    html=html.replace('近似描画の小パッチ分割','曲面の分割').replace('曲面の法線（近似用）','曲面の法線で陰影').replace('ワイヤー／パッチ境界','ポリゴンの辺')
                    html=html.replace('<label>レベル <select','<label>比較元メッシュ <select')
                    html=html.replace('<option value="curve">LPF＋3次近似曲面</option>','<option value="curve">曲面（描画方式で選択）</option>')
                    html=html.replace('<option value="ray" selected>','<option value="bspline" selected>階層Bスプライン</option><option value="ray">')
                    html=html.replace('<button id="reset">','<label>Bスプラインの最細区間 <select id="splineQuality">'+''.join(f'<option value="{n}"'+(' selected' if n==128 else '')+f'>{SPAN/n:g} m</option>' for n in SIZES)+'</select></label><button id="reset">')
                    html=html.replace('<script src="/lpf_curve_compare.js">','<script src="/bspline_surface.js"></script><script src="/lpf_curve_compare.js">')
                    a=html.index('<div id="note">');b=html.index('</div>',a)
                    html=html[:a]+'<div id="note">左は元標高の三角形メッシュ、右は選択した曲面。Bスプラインは疎な階層補正を保存し、表示時は曲面からGPUでポリゴンを生成します。三角形一覧・元DEMは曲面表示に使いません。区間長はWebメルカトル上の値。ワイヤーは生成したポリゴンの辺です。分割数は形状の近似精度を変え、描画は400万面を上限とします。左ドラッグ:移動／右ドラッグ:回転／ホイール:拡大縮小。LOD・別地域との接合は未実装。</div>'+html[b+6:]
                    body=html.encode();mime='text/html; charset=utf-8'
                elif url.path in ['/lpf_curve_compare.js','/lpf_curve_raycast.js','/bspline_surface.js']:
                    body=Path(__file__).with_name(url.path[1:]).read_bytes();mime='text/javascript; charset=utf-8'
                else:self.send_error(404);return
                self.send_response(200);self.send_header('Content-Type',mime);self.send_header('Content-Length',str(len(body)));self.send_header('Cache-Control','no-cache')
                if encoding:self.send_header('Content-Encoding',encoding)
                self.end_headers();self.wfile.write(body)
            except Exception as e:self.send_error(400,str(e))
        def log_message(self,*args):pass
    print(f'http://127.0.0.1:{args.port}/',flush=True)
    ThreadingHTTPServer(('127.0.0.1',args.port),Handler).serve_forever()


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('action',choices=['generate','serve']);p.add_argument('--source',type=Path,default=Path('build/terrain-lod.sqlite3'));p.add_argument('--base',type=Path,default=Path('build/lpf-curve-smooth-prototype/comparison.sqlite3'));p.add_argument('--output',type=Path,default=Path('build/bspline-surface-prototype-v3'));p.add_argument('--port',type=int,default=61467)
    a=p.parse_args();dict(generate=generate,serve=serve)[a.action](a)
