"""完成済みRust階層曲面を既存WebGPUビューワーへ読み取り専用で配信する。"""
import argparse,json,math,re,sqlite3,struct
from pathlib import Path
from functools import lru_cache
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from urllib.parse import urlparse,parse_qs
import numpy as np
from scipy.spatial import Delaunay
from sibson_native_codec import decode_independent
ROOT=Path(__file__).resolve().parents[2]
ASSETS=Path(__file__).parent
WORLD=40075016.68557849
ASSET_MAP={'/control-gpu128.js':'natural_c1_128.js','/control-gpu128-render.wgsl':'natural_c1_128_render.wgsl','/control-gpu.wgsl':'natural_c1_128_compute.wgsl','/control-gpu128-selection.mjs':'sibson_gpu_variable_selection.mjs','/control-gpu128-patches.mjs':'natural_c1_128_patches.mjs','/control-gpu128-lod.mjs':'natural_c1_128_lod.mjs','/control-gpu128-morph.mjs':'natural_c1_128_morph.mjs'}
def header(info,body=b''):
 data=json.dumps(info,separators=(',',':'),ensure_ascii=False).encode();data+=b' '*(-len(data)%4)
 return struct.pack('<I',len(data))+data+body
class Dataset:
 def __init__(self,root,gpu_aux=None):
  self.gpu_aux=gpu_aux or (root.parent/'natural-xyz-hierarchy-gpu-v2' if root.name=='natural-xyz-hierarchy-v1' else root.with_name(root.name+'-gpu-v2'))
  self.root=root.resolve();self.settings=json.loads((root/'settings.json').read_text(encoding='utf-8-sig'));self.levels=sorted(self.settings['levels'],key=lambda a:a['z']);self.by_error={v['error_m']:v for v in self.levels};self.regions={v['region']:v for v in self.settings['regions']}
  with sqlite3.connect((self.root/'queue.sqlite3').as_uri()+'?mode=ro',uri=True) as c:self.ready=set(c.execute("select z,x,y from jobs where state='ready'"))
 def origin(self,region):return self.regions[region]['bounds'][:2]
 def folder(self,error,x,y):
  z=self.by_error[error]['z'];key=(z,x,y)
  if key not in self.ready:raise ValueError('未完成または存在しないタイル')
  return self.root/f'tiles/{z}/{x}/{y}'
 @lru_cache(maxsize=3)
 def manifest(self,region):
  origin=self.origin(region);bounds=self.regions[region]['bounds'];rows=[]
  for z,x,y in sorted(self.ready):
   side=WORLD/(1<<z);b=[x*side,y*side,(x+1)*side,(y+1)*side]
   if b[0]>=bounds[2] or b[2]<=bounds[0] or b[1]>=bounds[3] or b[3]<=bounds[1]:continue
   level=next(v for v in self.levels if v['z']==z);rows.append(dict(z=z,error=level['error_m'],x=x,y=y,side=side,bounds=[b[i]-origin[i%2] for i in range(4)],state='ready'))
  roots=[[r['x'],r['y']] for r in rows if r['z']==self.levels[0]['z']]
  return dict(config=dict(title=f'Rust階層生成・128 km・最詳細z{self.levels[-1]["z"]}／{self.levels[-1]["error_m"]:g} m・上方向許容×{self.settings.get("upwardErrorRatio",1):g}',levels=[dict(z=v['z'],error=v['error_m'],side=v['side_m']) for v in self.levels],roots=roots,clipBounds=[0,0,128000,128000],span=128000,lpf=False,generationMode='fine-to-coarse'),tiles=rows)
 def metadata(self,region,error,x,y):
  folder=self.folder(error,x,y);aux=self.gpu_aux/f'tiles/{self.by_error[error]["z"]}/{x}/{y}';info=json.loads(((aux if (aux/'repack.json').exists() else folder)/'packet.json').read_text());origin=self.origin(region);side=self.by_error[error]['side_m'];z=self.by_error[error]['z'];core=[x*side,y*side,(x+1)*side,(y+1)*side]
  info.update(region=region,side=side,origin=[(core[0]+core[2])/2-origin[0],(core[1]+core[3])/2-origin[1]],coreBounds=[core[i]-origin[i%2] for i in range(4)],worldFieldBounds=[core[i]-origin[i%2]+(-250 if i<2 else 250) for i in range(4)])
  # 計算用ガードの近くでは勾配が急変するため、PNG評価格子2画素ぶんを
  # 表示キャッシュの外周から除く。保存曲面と探索木は変更しない。
  inset=2*WORLD/4194304
  padding=250-inset
  info.update(bounds=[-side/2-padding,-side/2-padding,side/2+padding,side/2+padding],worldFieldBounds=[core[i]-origin[i%2]+(-padding if i<2 else padding) for i in range(4)],displayFieldInset=inset,displayFieldPadding=padding)
  if info.get('empty'):info['format']='empty-terrain-v1'
  return folder,info
 @lru_cache(maxsize=96)
 def packet(self,region,error,x,y,direct=False):
  folder,info=self.metadata(region,error,x,y)
  if info.get('empty') or direct:
   if direct and not info.get('empty'):info['format']='control-metadata-v1'
   return header(info)
  aux=self.gpu_aux/f'tiles/{self.by_error[error]["z"]}/{x}/{y}'
  data=((aux if (aux/'repack.json').exists() else folder)/'packet.bin').read_bytes();length=struct.unpack_from('<I',data)[0]
  # XYZの大きな絶対座標はGPUへ渡さず、各地域の左上を原点にする。
  return header(info,data[4+length:])
 @lru_cache(maxsize=64)
 def wire(self,region,error,x,y,mesh=False):
  folder,info=self.metadata(region,error,x,y)
  if info.get('empty'):points=np.empty((0,3));indices=np.empty(0,dtype='<u4')
  else:
   points=decode_independent((folder/'control-xyz.nqc1').read_bytes()).astype(float);z=self.by_error[error]['z'];side=WORLD/(1<<z);core=np.array([x*side,y*side,(x+1)*side,(y+1)*side]);origin=np.asarray(self.origin(region));points[:,:2]=(points[:,:2]+np.floor(core[:2]/.0625+.5))*.0625-origin;points[:,2]*=.125
   faces=Delaunay(points[:,:2]).simplices;local=points[:,:2]-np.asarray(info['origin']);keep=np.all(abs(local)<=side/2+251,axis=1);remap=np.full(len(points),-1,dtype=np.int64);remap[keep]=np.arange(keep.sum())
   if mesh:indices=remap[faces[keep[faces].all(axis=1)]].astype('<u4').ravel()
   else:
    edges=np.concatenate([faces[:,[0,1]],faces[:,[1,2]],faces[:,[2,0]]]);edges=edges[keep[edges].all(axis=1)];indices=np.unique(np.sort(remap[edges],axis=1),axis=0).astype('<u4').ravel()
   points=points[keep]
  points=points.astype('<f4');return header(dict(points=len(points),indices=len(indices),source='stored-control-points',includesHalo=True),points.tobytes()+indices.tobytes())
def html_page(dataset):
 text=(ASSETS/'natural_c1_128.html').read_text(encoding='utf-8');text=re.sub(r'<title>.*?</title>','<title>Rust階層生成・標準XYZ・128 km</title>',text)
 text=re.sub(r'<select id="source">.*?</select>','<select id="source"><option value="hierarchy">Rust階層生成（完成済み）</option></select>',text)
 text=re.sub(r'<select id="quality">.*?</select>','<select id="quality"><option value="auto">自動LOD</option>'+''.join(f'<option value="{v["error_m"]:g}">z{v["z"]}・{v["error_m"]:g} m・{v["side_m"]/1000:.2f} km</option>' for v in dataset.levels)+'</select>',text)
 text=re.sub(r'<details>.*?</details>',f'<details><summary>生成条件</summary><select id="seamTest"><option value="none">通常</option></select><p>富士山・槍ヶ岳・筑波山の128 km四方と接合余白。標準XYZのz{dataset.levels[-1]["z"]}～z{dataset.levels[0]["z"]}。高くなる側の許容誤差は各レベル表示値の{dataset.settings.get("upwardErrorRatio",1):g}倍。最詳細は元DEM、粗レベルは直前の細曲面から生成。LPFなし。完成後に元DEMの全有効画素中心で誤差を監査済み。C¹・接合・描画近似後の誤差上限は保証対象外。領域外は表示しません。制御点・補助情報をGPUへ送り、自然近傍曲面をGPUで評価します。</p><p>左ドラッグ：移動／右ドラッグ：回転・傾き／ホイール：拡大縮小。© 国土地理院</p></details>',text)
 text=text.replace('href="/progress"','href="http://127.0.0.1:61490/"').replace('value=".5"><span id="c1AmountValue">50%','value=".25"><span id="c1AmountValue">25%');return text

def render_shader():
 text=(ASSETS/'natural_c1_128_render.wgsl').read_text();text=text.replace('@location(5) @interpolate(linear) bary:vec3<f32> }','@location(5) @interpolate(linear) bary:vec3<f32>, @location(6) xy:vec2<f32> }').replace('out.bary=bary[id%3u];','out.bary=bary[id%3u];out.xy=xy;')
 text=text.replace('@fragment fn fragment(v:Varying)->@location(0) vec4<f32>{','@fragment fn fragment(v:Varying)->@location(0) vec4<f32>{\n if(any(v.xy<vec2<f32>(0.))||any(v.xy>vec2<f32>(128000.))){discard;}')
 for name in ['wireFragment','controlMeshFragment','emptyFragment']:
  pattern=f'@fragment fn {name}(v:WireOutput)->@location(0) vec4<f32>{{';text=text.replace(pattern,pattern+'if(any(v.xy<vec2<f32>(0.))||any(v.xy>vec2<f32>(128000.))){discard;}')
 return text
class ViewerHTTPServer(ThreadingHTTPServer):
 # LOD切替で旧要求と新要求が重なる間も接続を受け付ける。
 request_queue_size=128
 daemon_threads=True

class Handler(BaseHTTPRequestHandler):
 def do_GET(self):
  try:
   url=urlparse(self.path);args=parse_qs(url.query);get=lambda k,d:args.get(k,[d])[0];dataset=self.server.dataset;region=get('region','fuji')
   if region not in dataset.regions:raise ValueError('不正な地域')
   if url.path=='/':data=html_page(dataset).encode();mime='text/html; charset=utf-8'
   elif url.path=='/variable-manifest':data=json.dumps(dataset.manifest(region)).encode();mime='application/json'
   elif url.path in ['/control-gpu128-packet','/control-wire']:
    error=float(get('error','640'));x=int(get('x','0'));y=int(get('y','0'))
    data=dataset.packet(region,error,x,y,get('direct','0')=='1') if url.path.endswith('packet') else dataset.wire(region,error,x,y,get('mesh','0')=='1');mime='application/octet-stream'
   elif url.path in ASSET_MAP:
    data=render_shader().encode() if url.path=='/control-gpu128-render.wgsl' else (ASSETS/ASSET_MAP[url.path]).read_bytes();mime='text/javascript; charset=utf-8' if url.path.endswith(('.js','.mjs')) else 'text/plain; charset=utf-8'
   else:self.send_error(404);return
   self.send_response(200);self.send_header('Content-Type',mime);self.send_header('Cache-Control','no-store');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
  except (BrokenPipeError,ConnectionResetError):pass
  except (ValueError,KeyError,OSError,sqlite3.Error) as e:self.send_error(400,str(e).encode('ascii','backslashreplace').decode())
 def log_message(self,*args):pass
if __name__=='__main__':
 parser=argparse.ArgumentParser();parser.add_argument('--port',type=int,default=61491);parser.add_argument('--output',type=Path,default=ROOT/'build/natural-xyz-hierarchy-z13-25-upquarter-local-v1');parser.add_argument('--gpu-aux',type=Path);a=parser.parse_args();server=ViewerHTTPServer(('127.0.0.1',a.port),Handler);server.dataset=Dataset(a.output,a.gpu_aux);server.serve_forever()
