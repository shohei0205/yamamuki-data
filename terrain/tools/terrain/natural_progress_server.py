import json,time,sqlite3,argparse
from pathlib import Path
from contextlib import closing
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
ROOT=Path(__file__).resolve().parents[2]
RUN=ROOT/'build/terrain-national-natural-20261029'
def read(p):
 try:return json.loads(p.read_text(encoding='utf-8-sig'))
 except (OSError,ValueError):return None
def snapshot():
 run=RUN
 for _ in range(8):
  nxt=read(run/'next-job.json')
  if not nxt:break
  target=Path(nxt['output']).resolve()
  if target.parent!=RUN.resolve().parent:break
  run=target
 names=['pipeline-progress','progress','coverage-progress','verification','pipeline-error']
 data={name:read((RUN if name.startswith('pipeline') else run)/(name+'.json')) for name in names}
 data['updated']=time.strftime('%Y-%m-%d %H:%M:%S');data['output']=str(run);data['levels']=[]
 db=run/'work.sqlite3'
 if db.exists():
  try:
   with closing(sqlite3.connect(db.resolve().as_uri()+'?mode=ro',uri=True,timeout=.3)) as c:
    meta=dict(c.execute('SELECT key,value FROM metadata'))
    for z,e in [(8,800),(9,400),(10,200),(11,100),(12,50),(13,25),(14,12.5)]:
     data['levels'].append(dict(z=z,error=e,phase=meta.get(f'fit-phase-z{z}','未開始'),points=meta.get(f'point-count-z{z}',meta.get(f'point-count-{z}','—')),sigma=meta.get(f'lpf-sigma-z{z}','—')))
  except sqlite3.Error as ex:data['dbNote']=str(ex)
 return data
HTML='''<!doctype html><meta charset="utf-8"><title>全国地形生成 進捗</title><style>body{font:16px system-ui;background:#f3f6fa;margin:30px;color:#243144}section{background:white;padding:20px;border-radius:10px;margin:15px 0}table{border-collapse:collapse;width:100%}td,th{text-align:left;padding:9px;border-bottom:1px solid #ddd}pre{white-space:pre-wrap}h1{font-size:24px}</style><h1>全国地形生成・自然近傍補間</h1><div id="view">取得中</div><script>
const esc=x=>String(x??'—').replace(/[&<>\"]/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c]));
async function update(){try{const d=await(await fetch('/api/progress',{cache:'no-store'})).json();document.getElementById('view').innerHTML='<p>'+esc(d.updated)+' ／ 3秒ごとに更新</p><section><h2>実行状態</h2><pre>'+esc(JSON.stringify(d['pipeline-progress'],null,2))+'</pre><h3>現在の工程</h3><pre>'+esc(JSON.stringify(d.progress,null,2))+'</pre></section><section><h2>元DEMの取得確認</h2><pre>'+esc(JSON.stringify(d['coverage-progress'],null,2))+'</pre></section><section><h2>レベル別</h2><table><tr><th>レベル</th><th>許容誤差 m</th><th>LPF σ m</th><th>状態</th><th>制御点数</th></tr>'+d.levels.map(r=>'<tr>'+[r.z,r.error,r.sigma,r.phase,r.points].map(x=>'<td>'+esc(x)+'</td>').join('')+'</tr>').join('')+'</table></section><section><h2>検証結果</h2><pre>'+esc(JSON.stringify(d.verification,null,2))+'</pre><p>'+esc(d.output)+'</p></section>'+(d['pipeline-error']?'<section><h2>エラー記録</h2><pre>'+esc(JSON.stringify(d['pipeline-error'],null,2))+'</pre></section>':'')}catch(e){document.getElementById('view').textContent='再取得中: '+e}}update();setInterval(update,3000);
</script>'''
class Handler(BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path.split('?')[0] not in ('/','/api/progress'):self.send_error(404);return
  api=self.path.startswith('/api/');body=(json.dumps(snapshot(),ensure_ascii=False) if api else HTML).encode();self.send_response(200);self.send_header('Content-Type','application/json; charset=utf-8' if api else 'text/html; charset=utf-8');self.send_header('Cache-Control','no-store');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
 def log_message(self,*args):pass
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--port',type=int,default=61486);a=p.parse_args();ThreadingHTTPServer(('127.0.0.1',a.port),Handler).serve_forever()
