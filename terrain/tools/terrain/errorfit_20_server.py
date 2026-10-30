"""20～5120m試験用。旧61488と出力を分離。"""
import json,re,urllib.parse
from http.server import ThreadingHTTPServer
import natural_c1_128_server as base
from errorfit_20_series import OUT,LEVELS,connect,init,snapshot,folder
base.LEVELS=LEVELS;base.connect=connect;base.RUN=OUT
base.paths=folder
PRUNED=base.ROOT/'build/natural-error-pruned-v1'
base.DENSITY_ROOTS.update({'pruned':PRUNED,'prune-fine':PRUNED/'fine'})
def data_folder(region,error,x,y,density):
 if density in ['pruned','prune-fine'] and error==20 and x in [31,32] and y in [31,32]:
  candidate=(PRUNED if density=='pruned' else PRUNED/'fine')/region/('20' if density=='pruned' else '10')/f'{x}-{y}'
  if (candidate/'selection.json').exists():return candidate
 return base.paths(region,error,x,y)
base.data_folder=data_folder
def progress():
 d=snapshot();return dict(levels=d['levels'],active=d['active'],errors=d['failed'],errorfitFull=d)
base.progress=progress
old_progress_page=base.progress_page
base.progress_page=lambda data:old_progress_page(data).replace("全7品質","全9品質")
class Handler(base.Handler):
 def do_GET(self):
  url=urllib.parse.urlparse(self.path)
  if url.path not in ['/','/control-gpu128.js','/control-gpu128-render.wgsl']:return super().do_GET()
  if url.path=='/':
   text=(base.ASSETS/'natural_c1_128.html').read_text(encoding='utf-8')
   text=re.sub(r'<select id="quality">.*?</select>','<select id="quality"><option value="auto">自動LOD</option>'+''.join(f'<option value="{e}">{e} m / {side/1000:g} km</option>' for e,side in LEVELS)+'</select>',text)
   text=re.sub(r'<select id="source">.*?</select>','<select id="source"><option value="errorfit-full">元の20～5120 m</option><option value="prune-fine">削減前・10 m適合（中央4タイル）</option><option value="pruned">点削減・20 m上限（中央4タイル）</option></select>',text)
   text=re.sub(r'<details>.*?</details>','<details><summary>試験の条件</summary><select id="seamTest"><option value="none">通常</option></select><p>誤差20・40・80・160・320・640・1280・2560・5120 m。1280 m以上は4隅の有効DEM支持点から最大残差順に1点ずつ追加し、点数比を使わず誤差で終了。20～640 mの既存成果は旧初期点継承方式のままです。旧61488と同じ固定km境界の地域比較。削減比較は20 m・中央4タイルのみ。生成済みの範囲へ適用し、その他は元方式を表示します。削減前は10 m適合曲面、削減後は20 m上限。全国XYZ境界への移行ではありません。生成待ちは別品質へ偽装せず未生成として扱います。C¹・描画・接合後の誤差上限は保証しません。</p></details>',text)
  elif url.path.endswith('.js'):
   text=(base.ASSETS/'natural_c1_128.js').read_text(encoding='utf-8');text=text.replace("query.set('quality','12.5')","query.set('quality','20')")
  else:text=(base.ASSETS/'natural_c1_128_render.wgsl').read_text(encoding='utf-8').replace('/12.5','/20.')
  body=text.encode();self.send_response(200);self.send_header('Content-Type','text/html; charset=utf-8' if url.path=='/' else 'text/javascript; charset=utf-8' if url.path.endswith('.js') else 'text/plain; charset=utf-8');self.send_header('Cache-Control','no-store');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
if __name__=='__main__':init();ThreadingHTTPServer(('127.0.0.1',61489),Handler).serve_forever()
