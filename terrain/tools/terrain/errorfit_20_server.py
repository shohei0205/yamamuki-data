"""20～1280m試験用。旧61488と出力を分離。"""
import json,re,urllib.parse
from http.server import ThreadingHTTPServer
import natural_c1_128_server as base
from errorfit_20_series import OUT,LEVELS,connect,init,snapshot
base.LEVELS=LEVELS;base.connect=connect;base.RUN=OUT
base.paths=lambda region,error,x,y:OUT/region/f'{error:g}'/f'{x}-{y}'
base.data_folder=lambda region,error,x,y,density:base.paths(region,error,x,y)
def progress():
 d=snapshot();return dict(levels=d['levels'],active=d['active'],errors=d['failed'],errorfitFull=d)
base.progress=progress
class Handler(base.Handler):
 def do_GET(self):
  url=urllib.parse.urlparse(self.path)
  if url.path not in ['/','/control-gpu128.js','/control-gpu128-render.wgsl']:return super().do_GET()
  if url.path=='/':
   text=(base.ASSETS/'natural_c1_128.html').read_text(encoding='utf-8')
   text=re.sub(r'<select id="quality">.*?</select>','<select id="quality"><option value="auto">自動LOD</option>'+''.join(f'<option value="{e}">{e} m / {side/1000:g} km</option>' for e,side in LEVELS)+'</select>',text)
   text=re.sub(r'<select id="source">.*?</select>','<select id="source"><option value="errorfit-full">20～1280m・元DEM全画素へ誤差適合</option></select>',text)
   text=re.sub(r'<details>.*?</details>','<details><summary>試験の条件</summary><select id="seamTest"><option value="none">通常</option></select><p>誤差20・40・80・160・320・640・1280 m。旧61488と同じ固定km境界で誤差だけを比較。全国XYZ境界への移行ではありません。生成待ちは別品質へ偽装せず未生成として扱います。C¹・描画・接合後の誤差上限は保証しません。</p></details>',text)
  elif url.path.endswith('.js'):
   text=(base.ASSETS/'natural_c1_128.js').read_text(encoding='utf-8');text=re.sub(r'(?<![\d.])800(?![\d.])','1280',text);text=text.replace("query.set('quality','12.5')","query.set('quality','20')")
  else:text=(base.ASSETS/'natural_c1_128_render.wgsl').read_text(encoding='utf-8').replace('/12.5','/20.')
  body=text.encode();self.send_response(200);self.send_header('Content-Type','text/html; charset=utf-8' if url.path=='/' else 'text/javascript; charset=utf-8' if url.path.endswith('.js') else 'text/plain; charset=utf-8');self.send_header('Cache-Control','no-store');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
if __name__=='__main__':init();ThreadingHTTPServer(('127.0.0.1',61489),Handler).serve_forever()
