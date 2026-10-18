"""保存済みの DEM10B だけから、地域を選んで地形を表示する。"""
import argparse
import base64
import gzip
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import math
import sqlite3
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

import numpy as np
from ridges import RADIUS, TILE_SIZE, decode_png, lon_lat, tile_position
from compact_dem import decode as decode_compact, HEADER as COMPACT_HEADER

ROOT=Path(__file__).resolve().parents[2]
ZOOM=14


def choose_zoom(latitude, longitude, size_km, minimum=4, maximum=14):
    delta=math.degrees(size_km*500/RADIUS)
    west=max(-179.999,longitude-delta/math.cos(math.radians(latitude)))
    east=min(179.999,longitude+delta/math.cos(math.radians(latitude)))
    north=min(85,latitude+delta);south=max(-85,latitude-delta)
    for zoom in range(maximum,minimum-1,-1):
        x0,y0=tile_position(west,north,zoom);x1,y1=tile_position(east,south,zoom)
        if (x1-x0)*256*(y1-y0)*256<=800_000 or zoom==minimum:
            return zoom


def region_payload(cache, latitude, longitude, size_km, compact=None, database=None,requested_zoom=None,view=None,fixed=False):
    if not (-80<=latitude<=80 and -179<=longitude<=179 and 2<=size_km<=(4000 if database else 8)):
        raise ValueError('緯度・経度または表示範囲が不正です（範囲を確認してください）')
    if database:
        with sqlite3.connect(database.resolve().as_uri()+'?mode=ro',uri=True) as check:
            row=check.execute("SELECT value FROM metadata WHERE key='schemaFormat'").fetchone()
        check.close()
        if row and json.loads(row[0])=='yamamuki-terrain-mesh':
            from polygon_region import payload
            if requested_zoom is not None and requested_zoom not in list(range(4,15)):raise ValueError('縮尺が不正です')
            if fixed:
                if requested_zoom is None:raise ValueError('手動指定するレベルがありません')
                delta=math.degrees(size_km*500/RADIUS);dlon=delta/math.cos(math.radians(latitude))
                x0,y0=tile_position(max(-179.999,longitude-dlon),min(85,latitude+delta),requested_zoom)
                x1,y1=tile_position(min(179.999,longitude+dlon),max(-85,latitude-delta),requested_zoom)
                if (math.floor(x1)-math.floor(x0)+1)*(math.floor(y1)-math.floor(y0)+1)>256:raise ValueError('このレベルでは表示範囲が広すぎます。拡大してから選んでください。')
                zoom=requested_zoom;view=None
            else:
                budget=choose_zoom(latitude,longitude,size_km/2)
                zoom=min(requested_zoom,budget) if requested_zoom is not None else budget
            result=payload(database,latitude,longitude,size_km,zoom,view,outline=True)
            result.setdefault('coverage',{})['levelMode']='manual' if fixed else 'auto'
            return result
    delta_lat=math.degrees(size_km*500/RADIUS)
    delta_lon=delta_lat/math.cos(math.radians(latitude))
    zoom=choose_zoom(latitude,longitude,size_km) if database else ZOOM
    west,north=tile_position(max(-179.999,longitude-delta_lon),min(85,latitude+delta_lat),zoom)
    east,south=tile_position(min(179.999,longitude+delta_lon),max(-85,latitude-delta_lat),zoom)
    left,top,right,bottom=math.ceil(west*256),math.ceil(north*256),math.floor(east*256),math.floor(south*256)
    shape=(bottom-top+1,right-left+1)
    if shape[0]*shape[1]>1_600_000:
        raise ValueError('表示範囲が大きすぎます。範囲を狭くしてください')
    heights=np.full(shape,np.nan,dtype='<f4')
    saved=0;missing=0;quantization=0
    db=sqlite3.connect(database.resolve().as_uri()+'?mode=ro',uri=True) if database else None
    for y in range(top//256,bottom//256+1):
        for x in range(left//256,right//256+1):
            path=cache/str(ZOOM)/str(x)/f'{y}.png'
            packed=compact/str(ZOOM)/str(x)/f'{y}.ymd' if compact else None
            row=db.execute('SELECT data FROM tiles WHERE z=? AND x=? AND y=?',(zoom,x,y)).fetchone() if db else None
            if db:
                if row is None:
                    missing+=1
                    continue
                body=row[0];tile=decode_compact(body)
                quantization=max(quantization,COMPACT_HEADER.unpack_from(body)[3])
            elif packed and packed.exists():
                body=packed.read_bytes()
                tile=decode_compact(body)
                quantization=max(quantization,COMPACT_HEADER.unpack_from(body)[3])
            elif path.exists():
                tile=decode_png(path.read_bytes())
            else:
                missing+=1
                continue
            x0,y0=max(left,x*256),max(top,y*256)
            x1,y1=min(right+1,(x+1)*256),min(bottom+1,(y+1)*256)
            heights[y0-top:y1-top,x0-left:x1-left]=tile[y0-y*256:y1-y*256,x0-x*256:x1-x*256]
            saved+=1
    if db:db.close()
    finite=np.isfinite(heights)
    if not finite.any():
        raise ValueError('この範囲には保存済みの標高がありません。取得範囲の地図から別の場所を選んでください')
    offset=(1-2**(zoom-14))/2
    center_lon,center_lat=lon_lat((left+right)/2+offset,(top+bottom)/2+offset,zoom)
    meters=2*math.pi*RADIUS*math.cos(math.radians(center_lat))/(256*2**zoom)
    return {'attribution':'地理院タイル（DEM10B）を加工して作成',
            'heightfield':{'schemaVersion':1,'originPixel':[left,top],'zoom':zoom,
                           'sampleLocation':'pixel','sampleOffsetPixels':offset,'shape':list(shape),'encoding':'gzip-float32-le-base64',
                           'values':base64.b64encode(gzip.compress(heights.tobytes(),mtime=0)).decode('ascii'),
                           'center':[center_lon,center_lat,float(np.median(heights[finite]))],'step':1},
            'meshStepMeters':meters,'span':max(shape[0]-1,shape[1]-1)*meters,'methods':{},
            'coverage':{'savedTiles':saved,'missingTiles':missing,'validSamples':int(finite.sum()),
                        'totalSamples':heights.size,'latitude':latitude,'longitude':longitude,'sizeKm':size_km,
                        'heightQuantizationMeters':quantization,'zoom':zoom,'overview':zoom<14}}


def terrain_html(payload):
    viewer=Path(__file__).with_name('viewer.html').read_text('utf-8')
    # 尾根抽出はこの画面では行わず、標高から連続する地形を描く。
    viewer=viewer.replace('<option value="watershed">尾根抽出の比較：流域境界</option>','').replace('<option value="inverse-flow">尾根抽出の比較：反転流路</option>','')
    viewer=viewer.replace('<option value="shaded">','<option value="shaded" selected>')
    viewer=viewer.replace('<label>線の最小長 <input id="length"', '<label hidden>線の最小長 <input id="length"')
    viewer=viewer.replace('document.body.appendChild(renderer);','document.body.appendChild(renderer);\n  parent.postMessage({type:"terrain-ready",coverage:data.coverage,triangles:data.indices.length/3},location.origin);')
    viewer=viewer.replace('.catch(error=>{document.getElementById("error")', '.catch(error=>{parent.postMessage({type:"terrain-error",message:error.message},location.origin);document.getElementById("error")')
    return viewer.replace('/*DATA*/',json.dumps(payload,ensure_ascii=False,allow_nan=False).replace('<','\\u003c'))


def handler_class(cache, coverage, compact=None, database=None):
    from sqlite_relay import relay
    assets=Path(__file__).parent
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            url=urlsplit(self.path);status=200;encoding=None;content_type='text/html; charset=utf-8'
            try:
                if url.path in ('/','/index.html'):body=(assets/'region.html').read_bytes()
                elif url.path=='/tin-preview' and (ROOT/'build/terrain-tin.html').exists():body=(ROOT/'build/terrain-tin.html').read_bytes()
                elif url.path=='/qem-preview':
                    level=parse_qs(url.query).get('z',['11'])[0]
                    if level!='reference' and level not in {str(z) for z in range(4,15)}:raise ValueError('比較用レベルが不正です')
                    name='reference.html' if level=='reference' else 'level-'+level+'.html'
                    target=ROOT/'build/terrain-qem-final'/name
                    if not target.exists():self.send_error(404,'QEM preview is not generated');return
                    body=target.read_bytes()
                elif url.path=='/favicon.ico':status=204;body=b''
                elif url.path=='/terrain':
                    # HTML はデータを埋め込まず直ちに返す。SQL の問い合わせもブラウザーが行う。
                    bootstrap={'bootstrap':'/terrain-data?'+url.query+'&stage=overview','sqliteRelay':True}
                    html=terrain_html(bootstrap).replace('<script>','<script src="/terrain-client-bridge.js"></script>\n<script>',1)
                    body=html.encode('utf-8')
                elif url.path.startswith('/sqlite/'):
                    if not database:raise ValueError('地形の SQLite がありません')
                    body,content_type=relay(url.path,parse_qs(url.query),database,ROOT)
                elif url.path=='/sqlite-peaks':
                    body=(ROOT/'build/terrain-peaks.json.gz').read_bytes();encoding='gzip';content_type='application/json; charset=utf-8'
                elif url.path in ('/terrain-client-bridge.js','/terrain-client-worker.js','/terrain-client-core.js'):
                    body=(assets/url.path[1:]).read_bytes();content_type='text/javascript; charset=utf-8'
                elif url.path.startswith('/vendor/') and '/' not in url.path[len('/vendor/'):]:
                    name=url.path[len('/vendor/'):]
                    if name not in {p.name for p in (assets/'vendor').iterdir() if p.is_file()}:self.send_error(404);return
                    body=(assets/'vendor'/name).read_bytes();content_type='text/javascript; charset=utf-8' if name.endswith('.js') else 'text/plain; charset=utf-8'
                elif url.path=='/coverage.html' and coverage.exists():
                    body=coverage.read_text('utf-8')
                    extra='''<script>let selectionStart=null;canvas.addEventListener('pointerdown',e=>{selectionStart=point(e);});canvas.addEventListener('pointerup',e=>{const [a,b]=point(e);if(!selectionStart||Math.hypot(a-selectionStart[0],b-selectionStart[1])>5)return;const x=Math.floor((a-ox)/scale),y=Math.floor((b-oy)/scale);if(index.get(x+','+y)!=='saved')return;const lon=(x+.5)/2**14*360-180,lat=Math.atan(Math.sinh(Math.PI*(1-2*(y+.5)/2**14)))*180/Math.PI;parent.postMessage({type:'terrain-location',lat,lon},location.origin);});</script>'''
                    body=body.replace('</html>',extra+'</html>').encode('utf-8')
                elif url.path=='/terrain-data':
                    status=410;content_type='application/json; charset=utf-8';body=json.dumps({'error':'地形の処理はブラウザー側へ移動しました。/terrain を開いてください。'},ensure_ascii=False).encode('utf-8')
                else:self.send_error(404);return
            except (KeyError,ValueError) as error:
                status=422;content_type='application/json; charset=utf-8';body=json.dumps({'error':str(error)},ensure_ascii=False).encode('utf-8')
            except Exception:
                import traceback
                traceback.print_exc();status=500;content_type='application/json; charset=utf-8';body=json.dumps({'error':'保存データを取得できませんでした。'},ensure_ascii=False).encode('utf-8')
            self.send_response(status);self.send_header('Content-Type',content_type)
            if encoding:self.send_header('Content-Encoding',encoding)
            self.send_header('Cache-Control','no-store');self.send_header('Content-Length',str(len(body)))
            self.end_headers()
            try:self.wfile.write(body)
            except (BrokenPipeError,ConnectionAbortedError,ConnectionResetError):pass
        def log_message(self,*args):pass
    return Handler


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cache',type=Path,default=ROOT/'build/terrain-cache')
    parser.add_argument('--coverage',type=Path,default=ROOT/'build/terrain-national/coverage.html')
    parser.add_argument('--port',type=int,default=61434)
    parser.add_argument('--compact',type=Path,default=ROOT/'build/terrain-compact')
    parser.add_argument('--database',type=Path,default=ROOT/'build/terrain-lod.sqlite3')
    parser.add_argument('--mesh-database',type=Path,default=ROOT/'build/terrain-mesh.sqlite3')
    args=parser.parse_args()
    compact=args.compact if (args.compact/'manifest.json').exists() else None
    database=args.mesh_database if args.mesh_database.exists() else (args.database if args.database.exists() else None)
    server=ThreadingHTTPServer(('127.0.0.1',args.port),handler_class(args.cache,args.coverage,compact,database))
    print(f'http://127.0.0.1:{args.port}/',flush=True)
    server.serve_forever()
