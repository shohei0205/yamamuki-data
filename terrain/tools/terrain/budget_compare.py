"""20%余裕枠の比較を既存の地形ビューアーで配信する。"""
import argparse,json
from pathlib import Path
from http.server import ThreadingHTTPServer
from urllib.parse import urlsplit
import decimation_compare as viewer
viewer.OUT=viewer.ROOT/'build/additional-budget-20261024'
class Handler(viewer.Handler):
 def do_GET(self):
  name={'/':'budget_compare.html','/decimation_compare.js':'budget_compare.js'}.get(urlsplit(self.path).path)
  if name:
   self.send(Path(__file__).with_name(name).read_bytes(),'text/html; charset=utf-8' if name.endswith('html') else 'text/javascript; charset=utf-8')
  else:super().do_GET()
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--port',type=int,default=61459);a=p.parse_args();ThreadingHTTPServer(('127.0.0.1',a.port),Handler).serve_forever()
