"""一括曲面から切り出した制御点・セルをWebGPUへ送り、傾き方式を比較する。"""
import argparse,json,re,struct
from pathlib import Path
from urllib.parse import urlparse,parse_qs
from functools import lru_cache
from natural_hierarchy_viewer_server import ROOT,ASSETS,ASSET_MAP,ViewerHTTPServer,header,render_shader
from http.server import BaseHTTPRequestHandler
class Dataset:
 def __init__(self,root):self.root=root;self.regions={r:{} for r in ['fuji','yari','tsukuba']}
 def destination(self,region,error):return self.root/region if error==300 else self.root/region/'levels'/str(int(error))
 def available(self,region):
  path=self.root/'levels.json';levels=json.loads(path.read_text()) if path.exists() else [dict(z=8,error=300,tileSide=32000)]
  return [v for v in levels if (self.destination(region,v['error'])/'report.json').exists() and json.loads((self.destination(region,v['error'])/'report.json').read_text()).get('complete')]
 def manifest(self,region):
  levels=self.available(region);tiles=[]
  for v in levels:
   n=128000//v['tileSide'];side=v['tileSide']
   tiles.extend(dict(z=v['z'],error=v['error'],x=x,y=y,side=side,bounds=[x*side,y*side,(x+1)*side,(y+1)*side],state='ready') for y in range(n) for x in range(n))
  n=128000//levels[0]['tileSide']
  return dict(config=dict(title='128 km共通曲面・全品質比較（元制御点の統合）',levels=[dict(z=v['z'],error=v['error'],side=v['tileSide']) for v in levels],roots=[[x,y] for y in range(n) for x in range(n)],clipBounds=[0,0,128000,128000],span=128000,lpf=False),tiles=tiles)
 @lru_cache(maxsize=96)
 def packet(self,region,error,mode,x,y):
  if mode not in ['global','local','partition'] or error not in [v['error'] for v in self.available(region)]:raise ValueError('不正または未完成の比較設定')
  return (self.destination(region,error)/mode/f'{x}-{y}'/'packet.bin').read_bytes()
 def wire(self,region,error,mode,x,y,mesh):
  import numpy as np
  from scipy.spatial import Delaunay
  data=self.packet(region,error,mode,x,y);n=struct.unpack_from('<I',data)[0];info=json.loads(data[4:4+n]);a=np.frombuffer(data,dtype='<f4',offset=4+n,count=info['controls']*8).reshape(-1,8);p=a[:,:3].copy();p[:,:2]*=1000
  faces=Delaunay(p[:,:2]).simplices
  # 全域に散らばる支持点を表示範囲の面へ絞る。形状そのものの点は変えない。
  r=info['coreBounds'];q=p[faces,:2];keep=(q[:,:,0].max(axis=1)>=r[0])&(q[:,:,0].min(axis=1)<=r[2])&(q[:,:,1].max(axis=1)>=r[1])&(q[:,:,1].min(axis=1)<=r[3]);faces=faces[keep]
  if mesh:indices=faces.astype('<u4').ravel()
  else:indices=np.unique(np.sort(np.concatenate([faces[:,[0,1]],faces[:,[1,2]],faces[:,[2,0]]]),axis=1),axis=0).astype('<u4').ravel()
  return header(dict(points=len(p),indices=len(indices)),p.astype('<f4').tobytes()+indices.tobytes())
def html(dataset,region):
 t=(ASSETS/'natural_c1_128.html').read_text();t=re.sub(r'<title>.*?</title>','<title>全域曲面の分割・局所傾き比較</title>',t)
 t=re.sub(r'<select id="source">.*?</select>','<select id="source"><option value="global">一括曲面＋全域傾き（比較元）</option><option value="local">一括曲面＋共通近傍の傾き</option><option value="partition">作業領域分割＋共通近傍の傾き</option></select>',t)
 t=re.sub(r'<select id="quality">.*?</select>','<select id="quality">'+''.join(f'<option value="{v["error"]}">元z{v["z"]}・{v["error"]} m・保存{v["tileSide"]//1000} km</option>' for v in dataset.available(region))+'</select>',t)
 t=re.sub(r'<details>.*?</details>','<details open><summary>比較条件</summary><select id="seamTest"><option value="none">通常</option></select><p>三地点128 km・品質別の固定表示比較。既存の各品質の制御点を統合して全域接続を再構築し、品質別の大きさのタイルへ分割。DEMへの再適合・誤差保証の引継ぎではありません。同品質の高さブレンドなし。局所方式は共通Delaunay一周近傍から傾きを復元。作業領域は64 kmの2×2。GPU補間キャッシュは可視枚数に応じて16～512分割（同じ品質は共通）。自動LOD・混合品質はこの比較の対象外。境界線・ワイヤーで確認できます。© 国土地理院</p><p><a href="/report">測定結果</a> ／ <a href="/progress">生成進捗</a> ／ <a href="http://127.0.0.1:61491/">従来61491</a></p></details>',t)
 if hasattr(dataset,'database'):
  t=re.sub(r'<select id="source">.*?</select>','<select id="source"><option value="sqlite">SQLite→Rust復号・補助情報再生成</option><option value="partition">安定近傍・一括生成（比較元）</option><option value="legacy">旧局所方式（61492）</option></select>',t)
  t=t.replace('三地点128 km・品質別の固定表示比較。','20.83 MiBのSQLiteからサーバー側RustでXYZ復号・局所傾き・セル・GPU索引を再生成する比較。補助情報は派生キャッシュとして保存し、SQLite自体には含めません。共円格子の不定な対角線を傾き計算から除外し、切り出し方に依存しない近傍を使います。三地点128 km・品質別の固定表示比較。')
  t=t.replace('<select id="quality">','<select id="quality"><option value="auto">自動LOD（最詳細25 m）</option>',1)
  t=t.replace('自動LOD・混合品質はこの比較の対象外。','自動LODは画面誤差から品質を選び、異品質の境界だけを接合します。同品質は高さを混ぜません。')
  t=t.replace('三地点128 km・品質別の固定表示比較。','三地点128 km・固定品質／自動LODの比較。')
  t=t.replace('href="/report"','href="/sqlite-status"')
 return t
def sqlite_render_shader():
 t=render_shader()
 field="""fn fieldAt(q:vec2<f32>,t:Tile,old:bool)->vec3<f32>{
 if(all(q>=t.field.xy)&&all(q<=t.field.zw)){if(old){return oldSample(q,t);}return evaluate(q,t);}
 // 隣に同品質の実キャッシュがあれば外挿せず、その共通曲面を使う。
 for(var i=0u;i<t.data.w;i+=1u){let other=tiles[neighbors[t.data.z+i]];
  if(other.misc.x==t.misc.x&&all(q>=other.field.xy)&&all(q<=other.field.zw)){
   if(old){return oldSample(q,other);}return evaluate(q,other);
  }
 }
 let anchor=clamp(q,t.field.xy,t.field.zw);var value=vec3<f32>(0.);
 if(old){value=oldSample(anchor,t);}else{value=evaluate(anchor,t);}
 return vec3<f32>(value.x+dot(value.yz,q-anchor),value.yz);
}
"""
 t=re.sub(r'fn fieldAt\(.*?\nfn blendFields\(',lambda _:field+'fn mixedBlendFields(',t,flags=re.S)
 wrapper=(ASSETS/'natural_shared_auto.wgsl').read_text()
 t=t.replace('fn blended(',wrapper+'\nfn blended(',1)
 # 索引値の剰余ではなく格子の3彩色を使い、各三角形の重心座標を保つ。
 t=t.replace('out.bary=bary[id%3u]', 'out.bary=bary[(ij.x+ij.y)%3u]')
 # 同じ格子頂点を面ごとに再評価せず、索引で共有する。
 t=re.sub(r'let corners=array<vec2<u32>,6>.*?let patchUV=', 'let ij=vec2<u32>(id%(n+1u),id/(n+1u));let patchUV=',t,count=1)
 return t

class Handler(BaseHTTPRequestHandler):
 def do_GET(self):
  try:
   u=urlparse(self.path);q=parse_qs(u.query);get=lambda k,d:q.get(k,[d])[0];region=get('region','fuji');d=self.server.dataset
   if region not in d.regions:raise ValueError('不正な地域')
   if u.path=='/':data=html(d,region).encode();mime='text/html; charset=utf-8'
   elif u.path=='/variable-manifest':data=json.dumps(d.manifest(region)).encode();mime='application/json'
   elif u.path=='/report':data=(d.root/('summary-levels.json' if (d.root/'summary-levels.json').exists() else 'summary.json')).read_bytes();mime='application/json; charset=utf-8'
   elif u.path=='/sqlite-status' and hasattr(d,'status'):data=json.dumps(d.status(),ensure_ascii=False,indent=2).encode();mime='application/json; charset=utf-8'
   elif u.path=='/progress-json':data=(d.root/'progress.json').read_bytes();mime='application/json; charset=utf-8'
   elif u.path=='/progress':data='<meta charset="utf-8"><title>共通曲面・生成進捗</title><h2>共通曲面・全品質の生成進捗</h2><pre id="status"></pre><script>async function update(){try{const r=await fetch("/progress-json");document.getElementById("status").textContent=JSON.stringify(await r.json(),null,2)}catch(e){document.getElementById("status").textContent=String(e)}}update();setInterval(update,2000)</script>'.encode();mime='text/html; charset=utf-8'
   elif u.path in ['/control-gpu128-packet','/control-wire']:
    x=int(get('x','0'));y=int(get('y','0'));mode=get('density','global');error=float(get('error','300'))
    data=d.packet(region,error,mode,x,y) if u.path.endswith('packet') else d.wire(region,error,mode,x,y,get('mesh','0')=='1');mime='application/octet-stream'
   elif u.path in ASSET_MAP:
    if u.path=='/control-gpu128-render.wgsl':
     t=render_shader();t=re.sub(r'fn fieldAt\(.*?\nfn blended\(', 'fn fieldAt(q:vec2<f32>,t:Tile,old:bool)->vec3<f32>{if(old){return oldSample(q,t);}return evaluate(q,t);}\nfn blendFields(q:vec2<f32>,owner:Tile,old:bool)->vec3<f32>{if(owner.data.w>0u){let first=tiles[neighbors[owner.data.z]];if(all(first.core==owner.core)){return fieldAt(q,first,old);}}return fieldAt(q,owner,old);}\nfn blended(',t,flags=re.S);data=(sqlite_render_shader() if hasattr(d,'database') else t).encode()
    elif u.path=='/control-gpu128.js':
     t=(ASSETS/ASSET_MAP[u.path]).read_text();cache_grid="""const commonGridCache=new Map();
function gridN(t){
 const key=[$('region').value,t.error,span,...pan,yaw,tilt,innerWidth,innerHeight,$('meshResolution').value].join('/');
 if(commonGridCache.has(key))return commonGridCache.get(key);
 let count=0,needed=16;
 for(const tile of manifest.tiles){if(tile.error!==t.error||!visible(tile))continue;count++;const depth=visibleDepth(tile.bounds,project);needed=Math.max(needed,tile.side*innerWidth*.8660254/depth/2);}
 const budget=2**Math.floor(Math.log2(Math.max(16,Math.sqrt(40*1048576/(Math.max(1,count)*32))-1)));
 const n=Math.max(16,Math.min(Number($('meshResolution').value)||128,512,budget,2**Math.ceil(Math.log2(needed))));
 if(commonGridCache.size>64)commonGridCache.clear();commonGridCache.set(key,n);return n;
}
"""
     t=re.sub(r'function gridN\(t\)\{.*?\n',lambda _:cache_grid,t,count=1)
     t=re.sub(r'while\(list\.reduce.*?desired=list;', 'desired=list;',t,count=1)
     t=t.replace('for(const v of activeResident().values())best.set', 'for(const v of activeResident().values())if(v.tile.error===rootError()||v.tile.n===gridN(v.tile))best.set')
     t=t.replace('for(const v of all.values())best.set', 'for(const v of all.values())if(v.tile.error===rootError()||v.tile.n===gridN(v.tile))best.set')
     t=t.replace("v.tile.error===rootError()||committedTiles.has(key)","(v.tile.error===rootError()||committedTiles.has(key))&&(!sceneReadyToCommit||v.tile.error===rootError()||v.tile.n===gridN(v.tile))")
     if hasattr(d,'database'):
      t=t.replace('(neighbors.some(j=>drawn[j].provisional)?8:0)', '(neighbors.some(j=>drawn[j].provisional)?8:0)+(neighbors.every(j=>drawn[j].tile.error===v.tile.error)?16:0)')
      # 曲面キャッシュを64 MiBまで許可し、広域の32分割への早すぎる低下を避ける。
      t=t.replace('40*1048576','64*1048576')
      # 過去の視点・解像度の失敗を、現在の取得失敗として数え続けない。
      t=t.replace('const best=new Map();for(const v of activeResident().values())', 'const wantedCacheKeys=new Set([...wanted.values()].map(cacheKey));for(const key of failed.keys())if(!wantedCacheKeys.has(key)||resident.has(key))failed.delete(key);const best=new Map();for(const v of activeResident().values())',1)
      indexed="""const patchIndexBuffers=new Map();
function patchIndices(n){
 let buffer=patchIndexBuffers.get(n);if(buffer)return buffer;
 const indices=new Uint32Array(n*n*6);let at=0;
 for(let y=0;y<n;y++)for(let x=0;x<n;x++){const a=y*(n+1)+x,b=a+1,c=a+n+2,d=a+n+1;indices.set([a,b,c,a,c,d],at);at+=6;}
 buffer=makeBuffer(indices,GPUBufferUsage.INDEX);patchIndexBuffers.set(n,buffer);return buffer;
}
"""
      t=t.replace('const commonGridCache=new Map();',indexed+'const commonGridCache=new Map();')
      t=t.replace('pass.draw(patches[i].n**2*6,end-i,0,i);','pass.setIndexBuffer(patchIndices(patches[i].n),\"uint32\");pass.drawIndexed(patches[i].n**2*6,end-i,0,0,i);')
      matches="""function cacheMatches(v){
 const target=desired.find(t=>t.error===v.tile.error&&t.x===v.tile.x&&t.y===v.tile.y);
 const n=target?target.n:v.tile.error===rootError()?128:gridN(v.tile);
 return v.tile.n===n;
}
"""
      t=t.replace('const commonGridCache=new Map();',matches+'const commonGridCache=new Map();')
      t=t.replace('field:v.info.worldFieldBounds,provisional:', 'field:v.info.worldFieldBounds,sampleStep:Math.max(v.info.worldFieldBounds[2]-v.info.worldFieldBounds[0],v.info.worldFieldBounds[3]-v.info.worldFieldBounds[1])/v.tile.n,provisional:')
      t=t.replace('pairs.push({base,t,lo,big,fineError:', 'pairs.push({base,t,lo,big,fine,coarse,line,value,vertical,fineError:')
      t=t.replace('let hmax=0,gmax=0,mixed=0,', 'let worst=null;let hmax=0,gmax=0,mixed=0,')
      t=t.replace('if(axis===0)hmax=Math.max(hmax,d);', 'if(axis===0){if(d>hmax)worst={pair,values:Array.from(data.slice(k,k+12)),fine:sources[pair.fine],coarse:sources[pair.coarse],finePatch:options.patches?patches[pair.fine]:null,coarsePatch:options.patches?patches[pair.coarse]:null};hmax=Math.max(hmax,d);}')
      t=t.replace('return{samples:pairs.length,mixedSamples:mixed,', 'return{worst,samples:pairs.length,mixedSamples:mixed,')
      t=t.replace('v.tile.error===rootError()||v.tile.n===gridN(v.tile)','cacheMatches(v)')
      t=t.replace('!protectedKeys.has(k)&&v.tile.error!==rootError()','!protectedKeys.has(k)&&(v.tile.error!==rootError()||v.tile.n!==128)')
      t=t.replace('!keep.has(key)&&v.tile.error!==rootError()','!keep.has(key)&&(v.tile.error!==rootError()||v.tile.n!==128)')
     data=t.encode()
    elif u.path=='/control-gpu128-patches.mjs' and hasattr(d,'database'):
     # 6 px間隔では広域の尾根が失われるため、3 pxを目安にする。
     data=(ASSETS/ASSET_MAP[u.path]).read_text().replace('pixels/6','pixels/3').encode()
    else:data=(ASSETS/ASSET_MAP[u.path]).read_bytes()
    mime='text/javascript; charset=utf-8' if u.path.endswith(('.js','.mjs')) else 'text/plain; charset=utf-8'
   else:self.send_error(404);return
   self.send_response(200);self.send_header('Content-Type',mime);self.send_header('Cache-Control','no-store');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
  except (BrokenPipeError,ConnectionResetError):pass
  except Exception as e:self.send_error(400,str(e).encode('ascii','backslashreplace').decode())
 def log_message(self,*args):pass
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--port',type=int,default=61492);p.add_argument('--output',type=Path,default=ROOT/'build/natural-shared-128-v1');a=p.parse_args();s=ViewerHTTPServer(('127.0.0.1',a.port),Handler);s.dataset=Dataset(a.output);s.serve_forever()
