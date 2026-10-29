"""LPFなしC1・128kmタイルをローカル配信。生成要求は永続キューで優先。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
from pathlib import Path
from contextlib import closing
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
import json,urllib.parse,struct,html,sqlite3
from functools import lru_cache
from scipy.spatial import Delaunay
import numpy as np
from curve_padding_packet import packet_with_padding
from natural_c1_128 import ROOT,RUN,LEVELS,REGIONS,connect,paths,STEP
DENSITY_ROOTS={name:ROOT/f'build/natural-raw-c1-128-{name}-v1' for name in ['half','quarter','eighth','reduced']}
REGULARIZED=ROOT/'build/natural-raw-c1-128-regularized-v3'
CONTINUOUS=ROOT/'build/natural-raw-c1-128-continuous-v1'
REDUCED_ALL=ROOT/'build/natural-raw-c1-128-reduced-all-v1'
def data_folder(region,error,x,y,density):
 if density=='errorfit-full':
  candidate=ROOT/'build/natural-raw-c1-128-errorfit-full-v1'/region/f'{error:g}'/f'{x}-{y}'
  if (candidate/'selection.json').exists():return candidate
  return paths(region,error,x,y)
 if density=='errorfit':
  candidate=ROOT/'build/natural-raw-c1-128-errorfit-levels-v1'/region/f'{error:g}'/f'{x}-{y}'
  if (candidate/'selection.json').exists():return candidate
  return paths(region,error,x,y)
 if density in ('lpf05','lpf1'):
  candidate=ROOT/f'build/natural-raw-c1-128-{density}-v1'/region/f'{error:g}'/f'{x}-{y}'
  if (candidate/'selection.json').exists():return candidate
  return paths(region,error,x,y)
 if density=='regularized':
  candidate=REGULARIZED/region/f'{error:g}'/f'{x}-{y}'
  if (candidate/'selection.json').exists():return candidate
  return paths(region,error,x,y)
 if density=='continuous':
  candidate=CONTINUOUS/region/f'{error:g}'/f'{x}-{y}'
  if (candidate/'selection.json').exists():return candidate
  return paths(region,error,x,y)
 if density=='reduced':
  candidate=REDUCED_ALL/region/f'{error:g}'/f'{x}-{y}'
  if (candidate/'selection.json').exists():return candidate
 return (DENSITY_ROOTS[density]/region/f'{error:g}'/f'{x}-{y}') if error==800 and density in DENSITY_ROOTS else paths(region,error,x,y)
ASSETS=Path(__file__).parent
def manifest(region):
 rows=[]
 with closing(connect()) as c:
  for error,x,y,state in c.execute('SELECT error,x,y,state FROM jobs WHERE region=? ORDER BY error DESC,y,x',(region,)):
   side=dict(LEVELS)[error];rows.append(dict(error=error,x=x,y=y,side=side,bounds=[x*side,y*side,(x+1)*side,(y+1)*side],state=state))
 return dict(config=dict(levels=[dict(error=e,side=s) for e,s in LEVELS],span=128000,lpf=False,curve='Sibson-global-C1-per-halo-tile'),tiles=rows)
def packet(region,error,x,y,n,density="sibson",direct=False):
 if density not in ["sibson","continuous","regularized","lpf05","lpf1","errorfit","errorfit-full",*DENSITY_ROOTS]:raise ValueError("不正な点数設定")
 if error not in dict(LEVELS) or not 8<=n<=512:raise ValueError('不正なタイル指定')
 count=round(128000/dict(LEVELS)[error])
 if not (0<=x<count and 0<=y<count):raise ValueError('タイル範囲外')
 with closing(connect()) as c,c:
  row=c.execute('SELECT state,info FROM jobs WHERE region=? AND error=? AND x=? AND y=?',(region,error,x,y)).fetchone()
  if not row:raise ValueError('タイルがありません')
  if row[0]!='ready':
   if row[0]=='pending':c.execute('UPDATE jobs SET priority=min(priority,?) WHERE region=? AND error=? AND x=? AND y=?',(-100+LEVELS.index((error,dict(LEVELS)[error])),region,error,x,y))
   raise ValueError('生成中です' if row[0]!='failed' else json.loads(row[1]).get('error','生成失敗'))
 folder=data_folder(region,error,x,y,density);meta=json.loads((folder/'packet.json').read_text())
 if meta.get('empty'):
  meta.update(format='empty-terrain-v1');header=json.dumps(meta).encode();header+=b' '*((-len(header))%4);return struct.pack('<I',len(header))+header
 if direct:
  info=dict(meta,format='control-metadata-v1');header=json.dumps(info,separators=(',',':')).encode();header+=b' '*((-len(header))%4);return struct.pack('<I',len(header))+header
 return packet_with_padding(folder,meta,n)
@lru_cache(maxsize=128)
def control_wire(region,error,x,y,density="sibson",mesh=False):
 folder=data_folder(region,error,x,y,density);info=json.loads((folder/'packet.json').read_text())
 if info.get('empty'):points=np.empty((0,3));edges=np.empty(0,dtype=np.uint32)
 else:
  with np.load(folder/'control-gradient.npz') as source:all_points=source['points']
  halo=info['halo'];side=info['side'];keep=np.all(np.abs(all_points[:,:2])<=side/2+halo+1,axis=1);remap=np.full(len(keep),-1,dtype=np.int64);remap[keep]=np.arange(keep.sum());faces=Delaunay(all_points[:,:2]).simplices
  if mesh:edges=remap[faces[keep[faces].all(axis=1)]].astype('<u4').ravel()
  else:
   edges=np.concatenate([faces[:,[0,1]],faces[:,[1,2]],faces[:,[2,0]]]);edges=edges[keep[edges].all(axis=1)];edges=np.unique(np.sort(remap[edges],axis=1),axis=0).astype('<u4').ravel()
  points=all_points[keep].copy();points[:,:2]+=info['origin']
 points=points.astype('<f4');meta=dict(points=len(points),indices=len(edges),source='stored-control-points',includesHalo=True);header=json.dumps(meta).encode();header+=b' '*((-len(header))%4);return struct.pack('<I',len(header))+header+points.tobytes()+edges.tobytes()
def progress_page(data):
 groups={}
 for row in data['levels']:groups.setdefault((row['region'],row['error']),{})[row['state']]=row['count']
 ready=sum(row['count'] for row in data['levels'] if row['state']=='ready');total=sum(row['count'] for row in data['levels']);rows=[]
 for (region,error),counts in groups.items():rows.append('<tr>'+''.join('<td>'+html.escape(str(v))+'</td>' for v in [region,error,counts.get('ready',0),sum(counts.values()),counts.get('running',0),counts.get('failed',0)])+'</tr>')
 full=data.get('errorfitFull',{});states=full.get('states',{});fullgroups={}
 for row in full.get('levels',[]):fullgroups.setdefault((row['region'],row['error']),{})[row['state']]=row['count']
 fullrows=''.join('<tr>'+''.join('<td>'+html.escape(str(v))+'</td>' for v in [region,error,counts.get('ready',0),sum(counts.values()),counts.get('running',0),counts.get('failed',0)])+'</tr>' for (region,error),counts in sorted(fullgroups.items(),key=lambda r:(r[0][0],-r[0][1])))
 fullhtml='<meta charset="utf-8"><meta http-equiv="refresh" content="5"><title>128km全域・誤差適合生成</title><style>body{font:16px system-ui;margin:30px}td,th{padding:8px;border-bottom:1px solid #ddd}pre{white-space:pre-wrap}</style><h1>128×128 km・全7品質の誤差適合</h1><a href="/?source=errorfit-full&quality=auto&c1Amount=0">ビューワー</a><p>3地域／'+str(states.get('ready',0))+' / '+str(full.get('total',16383))+' タイル完了／処理中 '+str(states.get('running',0))+'／失敗 '+str(states.get('failed',0))+'／5秒更新</p><table><tr><th>地域</th><th>品質 m</th><th>完了</th><th>計画</th><th>処理中</th><th>失敗</th></tr>'+fullrows+'</table><h2>現在の処理（PNG区画単位）</h2><pre>'+html.escape(json.dumps(full.get('active',[]),ensure_ascii=False,indent=2))+'</pre><h2>全域誤差適合の失敗</h2><pre>'+html.escape(json.dumps(full.get('failed',[]),ensure_ascii=False,indent=2))+'</pre>'
 return fullhtml+'<details><summary>従来版と旧比較処理の進捗</summary>'+'<meta charset="utf-8"><meta http-equiv="refresh" content="5"><title>LPFなしC1・128km進捗</title><style>body{font:16px system-ui;margin:30px}td,th{padding:8px;border-bottom:1px solid #ddd;text-align:left}pre{white-space:pre-wrap}</style><h1>LPFなし・C¹・128 km全品質生成</h1><a href="/">ビューワ</a><p>'+str(ready)+' / '+str(total)+' タイル完了（'+format(ready/max(total,1)*100,'.1f')+'%）／5秒更新</p><table><tr><th>地域</th><th>品質 m</th><th>完了</th><th>計画</th><th>処理中</th><th>失敗</th></tr>'+''.join(rows)+'</table><h2>元DEM全画素への誤差適合</h2><pre>'+html.escape(json.dumps(data.get('errorfit',{}),ensure_ascii=False,indent=2))+'</pre><h2>誤差計測による制御点削減</h2><pre>'+html.escape(json.dumps(data.get('reduction',{}),ensure_ascii=False,indent=2))+'</pre><h2>現在の処理</h2><pre>'+html.escape(json.dumps(data['active'],ensure_ascii=False,indent=2))+'</pre><h2>失敗</h2><pre>'+html.escape(json.dumps(data['errors'],ensure_ascii=False,indent=2))+'</pre>'+'</details>'

def progress():
 with closing(connect()) as c:
  levels=[dict(region=r,error=e,state=s,count=n) for r,e,s,n in c.execute('SELECT region,error,state,count(*) FROM jobs GROUP BY region,error,state ORDER BY region,error DESC,state')];active=[]
  for r,e,x,y in c.execute("SELECT region,error,x,y FROM jobs WHERE state='running'"):
   p=paths(r,e,x,y)/'progress.json';native=paths(r,e,x,y)/'progress.json';active.append(dict(region=r,error=e,x=x,y=y,progress=json.loads(p.read_text()) if p.exists() else {}))
  errors=[dict(region=r,error=e,x=x,y=y,message=json.loads(i).get('error')) for r,e,x,y,i in c.execute("SELECT region,error,x,y,info FROM jobs WHERE state='failed' LIMIT 10")]
 sources={r:((RUN/r/'source/progress.json').read_text() if (RUN/r/'source/progress.json').exists() else '準備待ち') for r,_,_ in REGIONS}
 reduced=REDUCED_ALL/'progress.json';reduction=json.loads(reduced.read_text()) if reduced.exists() else {}
 errorfit_path=ROOT/'build/natural-raw-c1-128-errorfit-levels-v1/progress.json';errorfit=json.loads(errorfit_path.read_text()) if errorfit_path.exists() else {}
 full_path=ROOT/'build/natural-raw-c1-128-errorfit-full-v1/progress.json';full=json.loads(full_path.read_text()) if full_path.exists() else {}
 return dict(levels=levels,active=active,errors=errors,sources=sources,reduction=reduction,errorfit=errorfit,errorfitFull=full)
class Handler(BaseHTTPRequestHandler):
 def do_GET(self):
  try:
   url=urllib.parse.urlparse(self.path);args=urllib.parse.parse_qs(url.query);get=lambda k,d:args.get(k,[d])[0];region=get('region','fuji')
   if region not in [r for r,_,_ in REGIONS]:raise ValueError('不正な地域')
   assets={'/':'natural_c1_128.html','/control-gpu128.js':'natural_c1_128.js','/control-gpu128-render.wgsl':'natural_c1_128_render.wgsl','/control-gpu.wgsl':'natural_c1_128_compute.wgsl','/control-gpu128-selection.mjs':'sibson_gpu_variable_selection.mjs','/control-gpu128-patches.mjs':'natural_c1_128_patches.mjs','/control-gpu128-lod.mjs':'natural_c1_128_lod.mjs'}
   if url.path in assets:
    p=ASSETS/assets[url.path];body=p.read_bytes();mime='text/html; charset=utf-8' if p.suffix=='.html' else 'text/javascript; charset=utf-8' if p.suffix in ['.js','.mjs'] else 'text/plain; charset=utf-8'
   elif url.path=='/variable-manifest':body=json.dumps(manifest(region)).encode();mime='application/json'
   elif url.path=='/control-gpu128-packet':body=packet(region,float(get('error','800')),int(get('x','0')),int(get('y','0')),int(get('n','128')),get('density','sibson'),get('direct','0')=='1');mime='application/octet-stream'
   elif url.path=='/control-wire':
    error=float(get('error','800'));x=int(get('x','0'));y=int(get('y','0'))
    if error not in dict(LEVELS) or not (0<=x<round(128000/dict(LEVELS)[error]) and 0<=y<round(128000/dict(LEVELS)[error])):raise ValueError('不正なタイル指定')
    body=control_wire(region,error,x,y,get('density','sibson'),get('mesh','0')=='1');mime='application/octet-stream'
   elif url.path=='/api/progress':body=json.dumps(progress(),ensure_ascii=False).encode();mime='application/json'
   elif url.path=='/progress':body=progress_page(progress()).encode();mime='text/html; charset=utf-8'
   else:self.send_error(404);return
   self.send_response(200);self.send_header('Content-Type',mime);self.send_header('Cache-Control','no-store');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
  except (ValueError,OSError,sqlite3.Error) as e:self.send_error(503,str(e).encode('ascii','backslashreplace').decode())
 def log_message(self,*args):pass
if __name__=='__main__':
 import sqlite3
 ThreadingHTTPServer(('127.0.0.1',61488),Handler).serve_forever()
