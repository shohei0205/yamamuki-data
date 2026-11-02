"""XYZ 3地域再生成の進捗を61490で配信。旧ビューワーは停止しない。"""
import json,argparse
from pathlib import Path
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from regional_xyz_tight import snapshot,OUT
ROOT=Path(__file__).resolve().parents[2]
ACTIVE=ROOT/"build/natural-active-generation.json"
def current_snapshot():
 root=Path(json.loads(ACTIVE.read_text(encoding="utf-8-sig"))["output"]) if ACTIVE.exists() else OUT
 data=json.loads((root/"progress.json").read_text(encoding="utf-8-sig")) if (root/"progress.json").exists() else snapshot()
 cfg=json.loads((root/"settings.json").read_text(encoding="utf-8-sig")) if (root/"settings.json").exists() else {}
 data["levelCount"]=len(cfg.get("levels",[]));data["output"]=str(root);data["generationMode"]=cfg.get("generationMode","independent")
 if (root/"generation-stop.json").exists():data["status"]="stopped-by-user"
 return data
HTML="""<!doctype html><meta charset=utf-8><title>3地域・XYZ全レベル再生成</title><style>body{font:15px sans-serif;margin:24px;background:#f5f7f8;color:#182724}table{border-collapse:collapse;background:white}td,th{padding:8px 18px;border:1px solid #ccd4d0}pre{white-space:pre-wrap;background:white;padding:16px}progress{width:100%;height:24px}</style><h2 id=heading>富士山・槍ヶ岳・筑波山：標準XYZ</h2><p id=description>進捗を読み込み中</p><progress id=bar></progress><p id=summary></p><table><thead><tr><th>z</th><th>最大誤差</th><th>完了</th><th>処理中</th><th>待機</th><th>失敗</th></tr></thead><tbody id=levels></tbody></table><h3>処理中</h3><pre id=active></pre><h3>失敗</h3><pre id=failures></pre><script>async function update(){try{let d=await(await fetch('/api/progress',{cache:'no-store'})).json();let s=d.states;heading.textContent='富士山・槍ヶ岳・筑波山：標準XYZ 全'+d.levelCount+'レベル';description.textContent=d.generationMode==='fine-to-coarse'?'128 km範囲＋接合余白。最詳細はDEM、粗レベルは一段細かい曲面から生成。元DEMは完成後に検査。':'各地点128 kmを覆う標準XYZタイルの独立生成';bar.max=d.total;bar.value=s.ready||0;summary.textContent=(d.status==='stopped-by-user'?'停止済み ／ ':'')+'完了 '+(s.ready||0)+' / '+d.total+' タイル・処理中 '+(s.running||0)+'・失敗 '+(s.failed||0)+'　'+new Date(d.updated*1000).toLocaleString();let rows=new Map();for(let a of d.levels){if(!rows.has(a.z))rows.set(a.z,{error:a.error});rows.get(a.z)[a.state]=a.count;}levels.innerHTML=[...rows].sort((a,b)=>b[0]-a[0]).map(([z,r])=>'<tr><td>'+z+'</td><td>'+r.error+' m</td><td>'+(r.ready||0)+'</td><td>'+(r.running||0)+'</td><td>'+(r.pending||0)+'</td><td>'+(r.failed||0)+'</td></tr>').join('');active.textContent=JSON.stringify(d.active,null,2);failures.textContent=JSON.stringify(d.failures,null,2);}catch(e){summary.textContent=String(e);}}update();setInterval(update,3000);</script>"""
class Handler(BaseHTTPRequestHandler):
 def do_GET(self):
  if self.path=='/api/progress':body=json.dumps(current_snapshot(),ensure_ascii=False).encode();mime='application/json'
  elif self.path=='/':body=HTML.encode();mime='text/html'
  else:self.send_error(404);return
  self.send_response(200);self.send_header('Content-Type',mime+'; charset=utf-8');self.send_header('Cache-Control','no-store');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
if __name__=='__main__':
 parser=argparse.ArgumentParser();parser.add_argument('--port',type=int,default=61490);a=parser.parse_args();ThreadingHTTPServer(('127.0.0.1',a.port),Handler).serve_forever()
