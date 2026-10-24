"""全国生成の進捗と保存済みデータ量を読み取り専用で表示する。"""
import json
import sqlite3
import time
from pathlib import Path
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
ROOT = Path(__file__).resolve().parents[2]
RUN = ROOT / 'build/terrain-national-xyz-20261024'
def read_json(path):
    return json.loads(path.read_text(encoding='utf-8-sig'))
def snapshot():
    data = read_json(ROOT / 'build/terrain-national-xyz-20261024.plan.json')
    data['run'] = read_json(RUN / 'progress.json') if (RUN / 'progress.json').exists() else None
    if data['run']:
        data['progressAgeSeconds'] = max(0, time.time() - (RUN / 'progress.json').stat().st_mtime)
    data['saved'] = []
    if data['run']:
        data['generationStarted'] = data['run'].get('generationStarted', False)
        child = data['run'].get('childProgress')
        if child:
            path = Path(child).resolve()
            if path.is_relative_to(RUN.resolve()) and path.exists():
                data['child'] = read_json(path)
    db_path = RUN / 'terrain.sqlite3'
    if db_path.exists():
        with sqlite3.connect(db_path.as_uri() + '?mode=ro', uri=True, timeout=1) as db:
            data['saved'] = [dict(zip(('z', 'tiles', 'vertices', 'triangles', 'blobBytes'), row)) for row in db.execute('SELECT z,count(*),sum(vertex_count),sum(triangle_count),sum(length(vertices)+length(triangles)) FROM tiles GROUP BY z')]
        data['sqliteBytes'] = db_path.stat().st_size
    return data
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        route = self.path.split('?', 1)[0]
        if route == '/':
            data = Path(__file__).with_name('national_progress.html').read_bytes()
            mime = 'text/html; charset=utf-8'
        elif route == '/api/progress':
            try:
                data = json.dumps(snapshot(), ensure_ascii=False).encode()
            except (OSError, ValueError, sqlite3.Error) as error:
                self.send_error(503, str(error)); return
            mime = 'application/json; charset=utf-8'
        else:
            self.send_error(404); return
        self.send_response(200)
        self.send_header('Content-Type', mime)
        self.send_header('Cache-Control', 'no-store')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)
    def log_message(self, *args):
        pass
if __name__ == '__main__':
    ThreadingHTTPServer(('127.0.0.1', 61462), Handler).serve_forever()
