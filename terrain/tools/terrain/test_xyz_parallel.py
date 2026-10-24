"""1領域処理と複数領域処理の確定形状・保存BLOBが一致することを確認する。"""
import argparse
import hashlib
import json
import sqlite3
import subprocess
import time
from pathlib import Path


def fingerprint(root):
    digest = hashlib.sha256()
    for file in sorted(root.glob('chunks/*/*.mesh')):
        digest.update(file.relative_to(root).as_posix().encode())
        digest.update(file.read_bytes())
    with sqlite3.connect(root / 'terrain.sqlite3') as db:
        for z, x, y, vb, tb in db.execute('SELECT z,tile_x,tile_y,vertices,triangles FROM tiles ORDER BY z,tile_y,tile_x'):
            digest.update(f'{z}/{x}/{y}'.encode())
            digest.update(vb)
            digest.update(tb)
    return digest.hexdigest()


def main(args):
    results = []
    for workers in (1, 4):
        root = args.output / f'workers-{workers}'
        if root.exists():
            raise ValueError('試験先は新しいディレクトリを指定してください')
        command = [str(args.exe.resolve()), '--national', '--national-chunk-cells', '512', '--national-workers', str(workers), '--national-window', *map(str, args.window), '--source', str(args.source.resolve()), '--source-sha256', args.sha256, '--output', str(root.resolve())]
        started = time.monotonic()
        process = subprocess.Popen(command)
        maximum_active = 0
        base_started = None
        base_seconds = None
        while process.poll() is None:
            try:
                progress = json.loads((root / 'progress.json').read_text())
                maximum_active = max(maximum_active, len(progress.get('activeChunks', [])))
                if progress['stage'] == 'generate-base' and base_started is None:
                    base_started = time.monotonic()
                elif progress['stage'] != 'generate-base' and base_started is not None and base_seconds is None:
                    base_seconds = time.monotonic() - base_started
            except (OSError, ValueError):
                pass
            time.sleep(0.05)
        if process.returncode:
            raise RuntimeError(f'生成失敗: workers={workers}')
        results.append(dict(workers=workers, seconds=time.monotonic() - started, baseSeconds=base_seconds, maximumActive=maximum_active, fingerprint=fingerprint(root)))
    assert results[0]['fingerprint'] == results[1]['fingerprint'], '並列数で形状・保存BLOBが変わりました'
    assert results[0]['maximumActive'] == 1
    assert results[1]['maximumActive'] > 1
    (args.output / 'comparison.json').write_text(json.dumps(results, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(results, indent=2), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--exe', type=Path, required=True)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--sha256', required=True)
    parser.add_argument('--window', nargs=4, type=int, required=True)
    main(parser.parse_args())
