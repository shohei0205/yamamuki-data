"""今回の4段階比較が保存されるのを待ち、データと描画を順に検証する。"""
import json, os, sqlite3, subprocess, time
from pathlib import Path
from verify_shared_allocation import verify
from uncut_mesh_topology import rebuild
from national_independent_mesh import ROOT


def main():
    pending = [48000, 96000, 384000, 768000]
    results = []
    deadline = time.monotonic() + 8 * 3600
    report = ROOT / 'build/terrain-shared-quality-verification.json'
    if report.exists():
        previous=json.loads(report.read_text(encoding='utf-8'))
        results=[r for r in previous.get('results',[]) if r.get('browserVerified') and r.get('topologyVersion')=='uncut-halo-v3' and r['totalSource']==int(r['mode'].split('-b')[1])]
        pending=[v for v in pending if v not in {r['totalSource'] for r in results}]
    node = Path.home() / '.cache/codex-runtimes/codex-primary-runtime/dependencies/node/bin/node.exe'
    while pending:
        if time.monotonic() > deadline:
            raise TimeoutError('比較メッシュの保存待ちが8時間を超えました')
        budget = pending[0]
        mode = f'terrain128-b{budget}'
        path = ROOT / f'build/terrain-shared-allocation-{mode}.sqlite3'
        ready = False
        if path.exists():
            try:
                with sqlite3.connect(path.resolve().as_uri() + '?mode=ro', uri=True, timeout=1) as db:
                    row=db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()
                    ready=bool(row) and json.loads(row[0])['sharedAllocation']['sourceVertices']==budget
            except sqlite3.OperationalError as error:
                if not any(word in str(error) for word in ['locked','no such table']):raise
        if not ready:
            time.sleep(5)
            continue
        print('検証開始', mode, flush=True)
        rebuild(path)
        result = verify(mode)
        result['topologyVersion']='uncut-halo-v3'
        result['sqliteBytes'] = path.stat().st_size
        env = os.environ.copy()
        env['NODE_PATH'] = str(node.parents[1] / 'node_modules')
        env['ALLOCATION_MODES'] = mode
        subprocess.run([str(node), 'tools/terrain/test_shared_allocation_view.cjs'], cwd=ROOT, env=env, check=True)
        result['browserVerified'] = True
        results.append(result)
        pending.pop(0)
        report.write_text(json.dumps(dict(complete=not pending, pending=pending, results=results), ensure_ascii=False, indent=2), encoding='utf-8')
        print('検証完了', mode, result['totalSource'], result['sqliteBytes'], flush=True)
    print('全4段階の生成物と描画の検証が完了しました', flush=True)


if __name__ == '__main__':
    main()
