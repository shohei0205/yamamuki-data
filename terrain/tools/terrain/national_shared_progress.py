"""現行全国共通曲面の進捗を読み取り専用で配信する。"""
import argparse
import csv
from contextlib import closing
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import io
import json
from pathlib import Path
import shutil
import sqlite3
import subprocess
import threading
import time

ROOT = Path(__file__).resolve().parents[2]


class Monitor:
    def __init__(self, run):
        self.run = run.resolve()
        self.cached = {}
        self.checked = 0
        self.lock = threading.Lock()
        self.vertices = {}
        self.history = []
        self.history_path = self.run/'monitor-history.json'
        try:
            self.history = json.loads(self.history_path.read_text(encoding='utf-8'))
        except (OSError, ValueError):
            pass
        threading.Thread(target=self.track_progress, daemon=True).start()
        threading.Thread(target=self.refresh_vertices, daemon=True).start()

    def track_progress(self):
        while True:
            try:
                p = self.read('progress.json')
                stage = p.get('stage', '')
                if stage in ('certify', 'certify-local'):
                    kind = '角度補修'
                    count = p.get('added', p.get('changes', 0))
                    reason = '最小角10°を確認中。点の追加・削除後は接続が変わるため再検査します。'
                elif stage == 'certify-repair':
                    kind = '誤差修復'
                    count = p.get('restored', 0)
                    reason = '全画素監査で許容誤差を超えたため、細レベルの点を復帰しています。'
                elif stage == 'audit':
                    kind = '全画素監査'
                    count = None
                    reason = ('許容誤差超過を検出。誤差修復後に角度を再検査します。'
                              if p.get('early_failure') else '角度確認後の曲面を元DEMと照合しています。')
                else:
                    time.sleep(1)
                    continue
                key = (p.get('z'), p.get('pass'), kind)
                previous = self.history[-1] if self.history else {}
                if (previous.get('z'), previous.get('pass'), previous.get('kind')) != key:
                    self.history.append(dict(z=p.get('z'), **{'pass': p.get('pass')},
                                             kind=kind, started=time.time()))
                row = self.history[-1]
                row.update(count=count, done=p.get('done', p.get('input')),
                           total=p.get('total', p.get('inputs')), reason=reason,
                           minimum_angle=p.get('minimum_angle'),
                           maximum_error=p.get('maximum_directional_error'),
                           updated=time.time())
                self.history = self.history[-100:]
                temporary = self.history_path.with_suffix('.tmp')
                temporary.write_text(json.dumps(self.history, ensure_ascii=False), encoding='utf-8')
                temporary.replace(self.history_path)
            except (OSError, ValueError):
                pass
            time.sleep(1)

    def refresh_vertices(self):
        while True:
            try:
                path = self.run/'work.sqlite3'
                with closing(sqlite3.connect(path.as_uri()+'?mode=ro', uri=True, timeout=.1)) as db:
                    levels = self.read('job.json')['levels']
                    counts = dict(self.vertices)
                    for level in levels:
                        z = level['z']
                        deadline = time.monotonic()+5
                        db.set_progress_handler(lambda: int(time.monotonic()>deadline), 1000)
                        try:
                            n, = db.execute('SELECT count(*) FROM active WHERE z=?', (z,)).fetchone()
                        except sqlite3.Error:
                            continue
                        if n:
                            counts[str(z)] = n
                        else:
                            counts.pop(str(z), None)
                        self.vertices = dict(counts)
            except (OSError, ValueError, sqlite3.Error):
                pass
            time.sleep(15)

    def read(self, name):
        return json.loads((self.run/name).read_text(encoding='utf-8-sig'))

    def metrics(self):
        if time.monotonic()-self.checked < 15:
            return self.cached
        with self.lock:
            if time.monotonic()-self.checked < 15:
                return self.cached
            data = dict(self.cached)
            try:
                launch = self.read('launch.json')
                result = subprocess.run(['tasklist', '/FI', f'PID eq {launch["pid"]}', '/FO', 'CSV', '/NH'], capture_output=True, timeout=3, creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
                rows = csv.reader(io.StringIO(result.stdout.decode('utf-8', errors='replace')))
                data['running'] = any(len(r)>1 and r[1] == str(launch['pid']) and r[0].casefold() == Path(launch['engine']).name.casefold() for r in rows)
            except (OSError, ValueError, subprocess.TimeoutExpired):
                data['running'] = None
            for name, kind in [('work.sqlite3', 'work'), ('terrain-shared.partial.sqlite3', 'packed'), ('terrain-shared.sqlite3', 'final')]:
                path = self.run/name
                if not path.exists():
                    continue
                try:
                    with closing(sqlite3.connect(path.as_uri()+'?mode=ro', uri=True, timeout=.1)) as db:
                        deadline = time.monotonic()+.15
                        db.set_progress_handler(lambda: int(time.monotonic()>deadline), 1000)
                        if kind == 'work':
                            data['stages'] = {k: json.loads(v) for k, v in db.execute("SELECT name,details FROM stages WHERE state='done'")}
                        else:
                            data['packed'] = {str(z): dict(tiles=n, points=p, bytes=b) for z,n,p,b in db.execute('SELECT z,count(*),sum(points),sum(length(payload)) FROM tiles GROUP BY z')}
                        data['countsCheckedAt'] = time.time()
                except (OSError, ValueError, sqlite3.Error):
                    # 集計で生成側の書込を待たせない。前回値を保持する。
                    pass
            self.checked = time.monotonic()
            self.cached = data
            return data

    def snapshot(self):
        job = self.read('job.json')
        progress = self.read('progress.json')
        launch = self.read('launch.json')
        files = {p.name: p.stat().st_size for p in self.run.glob('*.sqlite3*') if p.is_file()}
        return dict(progress=progress, ageSeconds=max(0,time.time()-(self.run/'progress.json').stat().st_mtime),
                    history=list(self.history), generation=job['generation_id'], levels=job['levels'], inputs=job['states'],
                    launch=dict(pid=launch['pid'], started=launch['started']),
                    metrics={**self.metrics(), 'vertices': self.vertices}, files=files, freeDiskBytes=shutil.disk_usage(self.run).free)

    def log(self, name):
        path = Path(self.read('launch.json')[name]).resolve()
        if not path.is_relative_to(self.run):
            raise ValueError('作業範囲外のログは読みません')
        with path.open('rb') as f:
            f.seek(max(0,path.stat().st_size-32768))
            return f.read().decode('utf-8', errors='replace').encode('utf-8')


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        route = self.path.split('?',1)[0]
        try:
            if route == '/':
                payload = Path(__file__).with_name('national_shared_progress.html').read_bytes()
                mime = 'text/html; charset=utf-8'
            elif route == '/api/progress':
                payload = json.dumps(self.server.monitor.snapshot(), ensure_ascii=False).encode()
                mime = 'application/json; charset=utf-8'
            elif route in ['/stdout', '/stderr']:
                payload = self.server.monitor.log(route[1:])
                mime = 'text/plain; charset=utf-8'
            else:
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header('Content-Type', mime)
            self.send_header('Cache-Control', 'no-store')
            self.send_header('Content-Length', str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
        except (BrokenPipeError, ConnectionResetError):
            pass
        except (OSError, ValueError, sqlite3.Error) as e:
            self.send_error(503, str(e).encode('ascii','backslashreplace').decode())

    def log_message(self, *args):
        pass


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', type=Path, default=ROOT/'build/national-shared-z6-z13-v1')
    parser.add_argument('--port', type=int, default=61494)
    args = parser.parse_args()
    server = ThreadingHTTPServer(('127.0.0.1', args.port), Handler)
    server.monitor = Monitor(args.run)
    server.serve_forever()
