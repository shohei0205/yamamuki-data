"""全国Qメッシュの進捗を読み取り専用で表示する。生成処理は行わない。"""
import argparse
import json
import sqlite3
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
STEM = ROOT / 'build/terrain-q4-q12-additional-go-free4096-20261023'
BASIC = ROOT / 'build/terrain-q4-q12-basic-rust-20261022'


def read_json(path):
    try:
        return json.loads(path.read_text(encoding='utf-8-sig'))
    except (OSError, ValueError):
        return None


def snapshot():
    selection = read_json(ROOT / 'build/q-progress-selection.json') or {}
    def selected_path(key, fallback):
        value = selection.get(key)
        if not isinstance(value, str):
            return fallback
        path = Path(value).resolve()
        if not path.is_relative_to((ROOT / 'build').resolve()):
            return fallback
        return path.with_suffix('')
    stem = selected_path('output', STEM)
    basic = selected_path('basic', BASIC)
    completed = {}
    try:
        with sqlite3.connect(stem.with_suffix('.sqlite3').as_uri() + '?mode=ro', uri=True, timeout=.5) as db:
            for key, value in db.execute("SELECT key,value FROM metadata WHERE key GLOB 'q*-complete'"):
                completed[key.split('-')[0][1:]] = json.loads(value)
    except (sqlite3.Error, OSError, ValueError):
        pass
    sizes = {}
    for name, path in [('basic', basic), ('additional', stem)]:
        sizes[name] = sum(p.stat().st_size for p in [path.with_suffix('.sqlite3'), Path(str(path) + '.sqlite3-wal')] if p.exists())
    additional = read_json(stem.with_suffix('.progress.json'))
    profile = read_json(stem.with_suffix('.profile.json'))
    if additional and additional.get('stage') == 'hierarchical-additional':
        q = additional.get('q')
        if isinstance(q, int) and 4 <= q <= 12:
            pool = stem.with_suffix('.work') / f'q{q}-additional'
            additional = read_json(pool.with_suffix('.progress.json')) or additional
            profile = read_json(pool.with_suffix('.profile.json')) or profile
    return dict(basic=read_json(basic.with_suffix('.progress.json')),
                additional=additional,
                profile=profile,
                completed=completed, storage=sizes, serverTime=time.time())


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        route = self.path.split('?', 1)[0]
        if route in ('/', '/progress.html'):
            body = Path(__file__).with_name('q_progress.html').read_bytes()
            mime = 'text/html; charset=utf-8'
        elif route == '/api/progress':
            body = json.dumps(snapshot(), ensure_ascii=False).encode('utf-8')
            mime = 'application/json; charset=utf-8'
        elif route == '/favicon.ico':
            self.send_response(204)
            self.end_headers()
            return
        else:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header('Content-Type', mime)
        self.send_header('Cache-Control', 'no-store')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_):
        pass


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--port', type=int, default=61453)
    args = parser.parse_args()
    ThreadingHTTPServer(('127.0.0.1', args.port), Handler).serve_forever()
