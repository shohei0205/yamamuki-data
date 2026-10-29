"""固定LPF地域比較。生成済みプレビューのみ読み取り配信する。"""
from pathlib import Path
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
import json,urllib.parse,sqlite3
from contextlib import closing
from functools import lru_cache
import numpy as np
from scipy.spatial import Delaunay
ROOT=Path(__file__).resolve().parents[2];DATA=ROOT/'build/natural-fixed-lpf-regions-v1';NEW=ROOT/'build/natural-fixed-lpf-fit8-regions-v1';SCALE=ROOT/'build/natural-scale-placement-regions-v1';ASSETS=Path(__file__).parent
BUDGET={f'budget{s}':ROOT/f'build/natural-budget-sigma{s}-v1' for s in [400,200,100,50]}
RAW=ROOT/'build/natural-raw-shape800-v1'
def base_for(variant):return RAW if variant in ['raw800','raw800c1'] else BUDGET.get(variant,SCALE if variant=='scale' else NEW if variant=='fit8' else DATA)
HTML='''<!doctype html><meta charset="utf-8"><title>固定LPF地域比較</title><style>body{margin:0;background:#e6f1fa;font:14px system-ui}canvas{width:100vw;height:100vh;display:block}header{position:absolute;top:8px;left:8px;right:8px;background:#fffffff0;padding:10px;border-radius:8px}select,button{margin:4px}.labels{position:absolute;bottom:10px;left:0;right:0;display:flex;pointer-events:none}.labels span{width:50%;text-align:center;background:#fffd}</style><canvas id="view"></canvas><header><b>固定LPF＋LPF基準適合・3地域比較</b><select id="region"><option value="fuji">富士山</option><option value="yari">槍ヶ岳</option><option value="tsukuba">筑波山</option></select><label>左<select id="left"></select></label><label>右<select id="right"></select></label><button id="reset">視点を戻す</button><label>制御点ワイヤー<select id="wire"><option value="none">なし</option><option value="overlay">曲面に重ねる</option><option value="only">ワイヤーのみ</option></select></label><label><input id="controls" type="checkbox">制御点</label><input id="net" type="checkbox" hidden><label>C¹適用量 <input id="c1Amount" type="range" min="0" max="1" step="0.05" value="0.5"><span id="c1AmountValue">50%</span></label><p id="status">生成結果を取得中</p><p>中心16 km。「全域傾き調整C¹」は保存XYZを維持して全点の勾配を同時調整。曲面から±1 m評価で精密陰影を計算します。適用量0%は通常自然近傍、100%は全域C¹。中間は両曲面の混合で、厳密なC¹連続性は保証しません。800 m上限の全画素監査は通常自然近傍に対する結果で、C¹曲面の誤差は表示格子で別測定。「LPFなし・誤差800 m」は元DEMの全有効画素で最大差800 m以内を検証し、初期の形状表現用点を保持します。「同一点数」は800 m版と点数をそろえ、LPFのσを400/200/100/50 mで比較。その他はσ＝品質値÷2。最新の一括配置はLPF幅と曲率から点間隔を決定し、最大誤差を満たすまでの反復追加はしません。誤差は表示格子上で事後測定。旧近似方式も保持。左ドラッグ：移動／右ドラッグ：回転／ホイール：拡大縮小。</p><p>表示は比較用512分割。表示用標高格子は配布形式ではありません。最大差は画面の表示格子上で元DEMと比較した参考値。<a href="/progress">地域生成の進捗</a></p></header><div class="labels"><span id="leftLabel"></span><span id="rightLabel"></span></div><script src="/weighted.js"></script>'''
@lru_cache(maxsize=64)
def control_mesh(region,error,variant,stamp):
 base=base_for(variant);folder=base/region
 z={800:8,400:9,200:10,100:11,50:12,25:13,12.5:14}[error]
 with closing(sqlite3.connect((folder/'work.sqlite3').resolve().as_uri()+'?mode=ro',uri=True,timeout=2)) as c:
  rows=np.asarray(c.execute('SELECT p.qx,p.qy,l.qz,l.flags FROM points p JOIN point_levels l ON l.id=p.id WHERE l.z=? ORDER BY p.id',(z,)).fetchall(),dtype=np.float64)
 if len(rows)<3:raise ValueError('制御点が不足しています')
 points=rows[:,:3]*[.0625,.0625,.125];faces=Delaunay(points[:,:2]).simplices
 keep=rows[:,3]!=1;indices=np.full(len(rows),-1,dtype=np.int64);indices[keep]=np.arange(keep.sum())
 edges=set()
 for face in faces:
  for a,b in [(face[0],face[1]),(face[1],face[2]),(face[2],face[0])]:
   if keep[a] and keep[b]:edges.add(tuple(sorted((int(indices[a]),int(indices[b])))))
 center=np.asarray(json.loads((folder/'region.json').read_text())['center']);local=points[keep].copy();local[:,:2]-=center
 return dict(points=local.tolist(),edges=np.asarray(sorted(edges),dtype=int).reshape(-1).tolist(),pointCount=int(keep.sum()),edgeCount=len(edges),note='保存制御点のXYZとDelaunay辺。世界外周の補助点は非表示。')
class Handler(BaseHTTPRequestHandler):
 def do_GET(self):
  try:
   u=urllib.parse.urlparse(self.path);q=urllib.parse.parse_qs(u.query);region=q.get('region',['fuji'])[0];error=float(q.get('error',['50'])[0]);variant=q.get('variant',['fit'])[0]
   if region not in ['fuji','yari','tsukuba'] or error not in [800,400,200,100,50,25,12.5] or variant not in ['dem','lpf','fit','fit8','scale','raw800','raw800c1',*BUDGET]:raise ValueError('不正な比較条件')
   if u.path=='/':body=HTML.encode();mime='text/html; charset=utf-8'
   elif u.path=='/weighted.js':body=(ASSETS/'fixed_lpf_compare.js').read_bytes();mime='text/javascript; charset=utf-8'
   elif u.path=='/weighted-field':
    base=base_for(variant);kind='c1' if variant=='raw800c1' else 'fit' if variant in ['fit8','scale','raw800','raw800c1',*BUDGET] else variant;path=base/region/f'{error:g}-{kind}.bin'
    if not (base/region/(f'preview-{error:g}-c1.json' if variant=='raw800c1' else f'preview-{error:g}.json')).exists():raise ValueError('この品質は生成中です。完了後に自動で表示します。')
    body=path.read_bytes();mime='application/octet-stream'
   elif u.path=='/available':
    availability={key:[e for e in [800,400,200,100,50,25,12.5] if (base/region/f'preview-{e:g}.json').exists()] for key,base in [('ready',DATA),('ready8',NEW),('readyScale',SCALE),('raw800',RAW),*BUDGET.items()]}
    availability['raw800c1']=[800] if (RAW/region/'preview-800-c1.json').exists() else []
    body=json.dumps(availability).encode();mime='application/json'
   elif u.path=='/control-mesh':
    if variant not in ['fit','fit8','scale','raw800','raw800c1',*BUDGET]:body=json.dumps(dict(points=[],edges=[],pointCount=0,edgeCount=0)).encode()
    else:
     path=base_for(variant)/region/f'preview-{error:g}.json'
     if not path.exists():raise ValueError('この品質の制御点は生成中です')
     body=json.dumps(control_mesh(region,error,variant,path.stat().st_mtime_ns)).encode()
    mime='application/json'
   elif u.path=='/weighted-report':
    variants={}
    for kind,base in [('fit',DATA),('lpf',DATA),('fit8',NEW),('scale',SCALE),('raw800',RAW),*BUDGET.items()]:
     path=base/region/f'preview-{error:g}.json'
     if not path.exists():continue
     r=json.loads(path.read_text());is_fit=kind!='lpf';variants[kind]=dict(controls=r['points'] if is_fit else 0,bytes=0,rawRmse=r.get('preview_fit_dem_rmse',0) if is_fit else r.get('preview_lpf_dem_rmse',0),rawMaxError=r['preview_fit_dem_max'] if is_fit else r['preview_lpf_dem_max'],fitError=r.get('fit_error_m',error),sigma=r['sigma'])
    c1=RAW/region/f'preview-{error:g}-c1.json'
    if c1.exists():
     r=json.loads(c1.read_text());variants['raw800c1']=dict(controls=r['points'],rawRmse=r['rawRmse'],rawMaxError=r['rawMaxError'],fitError=None,sigma=0,geometry=r['geometry'],controlPointMaximumError=r['control_point_max_error'])
    body=json.dumps(dict(variants=variants)).encode();mime='application/json'
   elif u.path=='/progress':
    rows=[]
    for tag,base in [('最新・LPFなし元DEM誤差800m',RAW),('LPFスケール一括配置',SCALE),('旧・近似誤差1/8（停止）',NEW),('旧・近似誤差1倍',DATA)]:
     for name in ['fuji','yari','tsukuba']:
      folder=base/name;progress=json.loads((folder/'progress.json').read_text()) if (folder/'progress.json').exists() else {};ready=[e for e in [800,400,200,100,50,25,12.5] if (folder/f'preview-{e:g}.json').exists()];rows.append(dict(mode=tag,region=name,ready=ready,progress=progress))
    body=('<meta charset="utf-8"><meta http-equiv="refresh" content="5"><title>地域生成進捗</title><a href="/">比較画面</a><pre>'+__import__('html').escape(json.dumps(rows,ensure_ascii=False,indent=2))+'</pre>').encode();mime='text/html; charset=utf-8'
   else:self.send_error(404);return
   self.send_response(200);self.send_header('Content-Type',mime);self.send_header('Content-Length',str(len(body)));self.send_header('Cache-Control','no-store');self.end_headers();self.wfile.write(body)
  except (ValueError,OSError,sqlite3.Error) as e:self.send_error(503,str(e).encode('ascii','backslashreplace').decode())
 def log_message(self,*a):pass
if __name__=='__main__':ThreadingHTTPServer(('127.0.0.1',61487),Handler).serve_forever()
