"""XYZ 3地域再生成の進捗を61490で配信。旧ビューワーは停止しない。"""
import json
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from regional_xyz_tight import snapshot,OUT,init
HTML="""<!doctype html><meta charset=utf-8><title>3地域・XYZ全レベル再生成</title><style>body{font:15px sans-serif;margin:24px;background:#f5f7f8;color:#182724}table{border-collapse:collapse;background:white}td,th{padding:8px 18px;border:1px solid #ccd4d0}pre{white-space:pre-wrap;background:white;padding:16px}progress{width:100%;height:24px}</style><h2>富士山・槍ヶ岳・筑波山：標準XYZ 全9レベル</h2><p>各地点128 km四方を覆うタイル全体を生成。誤差20～640 m、LPFなし、精細適合→不要点削除→全画素監査。</p><progress id=bar></progress><p id=summary></p><table><thead><tr><th>z</th><th>最大誤差</th><th>完了</th><th>処理中</th><th>待機</th><th>失敗</th></tr></thead><tbody id=levels></tbody></table><h3>処理中</h3><pre id=active></pre><h3>失敗</h3><pre id=failures></pre><script>async function update(){try{let d=await(await fetch('/api/progress',{cache:'no-store'})).json();let s=d.states;bar.max=d.total;bar.value=s.ready||0;summary.textContent='完了 '+(s.ready||0)+' / '+d.total+' タイル・処理中 '+(s.running||0)+'・失敗 '+(s.failed||0)+'　'+new Date(d.updated*1000).toLocaleString();let rows=new Map();for(let a of d.levels){if(!rows.has(a.z))rows.set(a.z,{error:a.error});rows.get(a.z)[a.state]=a.count;}levels.innerHTML=[...rows].sort((a,b)=>b[0]-a[0]).map(([z,r])=>'<tr><td>'+z+'</td><td>'+r.error+' m</td><td>'+(r.ready||0)+'</td><td>'+(r.running||0)+'</td><td>'+(r.pending||0)+'</td><td>'+(r.failed||0)+'</td></tr>').join('');active.textContent=JSON.stringify(d.active,null,2);failures.textContent=JSON.stringify(d.failures,null,2);}catch(e){summary.textContent=String(e);}}update();setInterval(update,3000);</script>"""
class Handler(BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path=='/api/progress':body=json.dumps(snapshot(),ensure_ascii=False).encode();mime='application/json'
  elif self.path=='/':body=HTML.encode();mime='text/html'
  else:self.send_error(404);return
  self.send_response(200);self.send_header('Content-Type',mime+'; charset=utf-8');self.send_header('Cache-Control','no-store');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
if __name__=='__main__':init();ThreadingHTTPServer(('127.0.0.1',61490),Handler).serve_forever()
