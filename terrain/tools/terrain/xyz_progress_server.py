"""200 km 地域生成の進捗を読み取り専用で表示する。"""
import argparse
import json
import time
import sqlite3
from pathlib import Path
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

ROOT = Path(__file__).resolve().parents[2]

def read(path):
    try:
        return json.loads(path.read_text(encoding="utf-8-sig"))
    except (OSError, ValueError):
        return None

def snapshot(output):
    regions = []
    for name in ("tsukuba", "yari", "fuji"):
        directory = output / name
        levels = []
        for z in range(14, 5, -1):
            report = read(directory / f"z{z}.report.json")
            old = read(ROOT / "build/xyz-regional-200km-20261024" / name / f"z{z}.report.json")
            levels.append(dict(z=z, report=report, previous=old))
        database = directory / "terrain.sqlite3"
        packing = read(directory / "packing-report.json") or []
        saved = {}
        try:
            with sqlite3.connect(database.as_uri() + "?mode=ro", uri=True, timeout=0.2) as db:
                for z, count, vertices, triangles, vb, tb in db.execute(
                    "SELECT z,count(*),sum(vertex_count),sum(triangle_count),sum(length(vertices)),sum(length(triangles)) FROM tiles GROUP BY z"):
                    saved[z] = dict(tiles=count, vertices=vertices, triangles=triangles, vertexBytes=vb, triangleBytes=tb)
        except sqlite3.Error:
            pass
        state_file = directory / "progress.json"
        state_age = max(0, time.time() - state_file.stat().st_mtime) if state_file.exists() else None
        regions.append(dict(name=name, packing=packing, saved=saved, stateAgeSeconds=state_age, progress=read(directory / "progress.json"), levels=levels,
                            sqliteBytes=database.stat().st_size if database.exists() else 0,
                            verification=read(directory / "independent-verification.json")))
    return dict(time=time.time(), run=read(output / "run.json"), regions=regions)

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        route = self.path.split("?", 1)[0]
        if route in ("/", "/progress.html"):
            body = Path(__file__).with_name("xyz_progress.html").read_bytes()
            mime = "text/html; charset=utf-8"
        elif route == "/api/progress":
            body = json.dumps(snapshot(self.server.output), ensure_ascii=False).encode()
            mime = "application/json; charset=utf-8"
        else:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Type", mime)
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *args):
        pass

if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=61460)
    parser.add_argument("--output", type=Path, default=ROOT / "build/xyz-regional-200km-parallel-20261024")
    args = parser.parse_args()
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    server.output = args.output.resolve()
    server.serve_forever()
