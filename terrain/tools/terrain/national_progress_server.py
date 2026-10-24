"""全国生成の計画と、未着手工程を読み取り専用で表示する。"""
import json
from pathlib import Path
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
ROOT=Path(__file__).resolve().parents[2]
class Handler(BaseHTTPRequestHandler):
 def do_GET(self):
  route=self.path.split('?',1)[0]
  if route=='/':data=Path(__file__).with_name('national_progress.html').read_bytes();mime='text/html; charset=utf-8'
  elif route=='/api/progress':
   path=ROOT/'build/terrain-national-xyz-20261024.plan.json'
   data=json.dumps(json.loads(path.read_text(encoding='utf-8-sig')),ensure_ascii=False).encode();mime='application/json; charset=utf-8'
  else:self.send_error(404);return
  self.send_response(200);self.send_header('Content-Type',mime);self.send_header('Cache-Control','no-store');self.send_header('Content-Length',str(len(data)));self.end_headers();self.wfile.write(data)
 def log_message(self,*args):pass
if __name__=='__main__':ThreadingHTTPServer(('127.0.0.1',61462),Handler).serve_forever()
