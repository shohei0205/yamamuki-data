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
        command = [str(args.exe.resolve()), '--national', '--national-chunk-cells', str(args.cells), '--national-workers', str(workers), '--national-boundary-workers', str(workers), '--national-pack-workers', str(workers), '--national-window', *map(str, args.window), '--source', str(args.source.resolve()), '--source-sha256', args.sha256, '--output', str(root.resolve())]
        started = time.monotonic()
        process = subprocess.Popen(command)
        maximum_active = 0
        base_started = None
        base_seconds = None
        sync_started = None
        sync_seconds = 0
        sync_active = 0
        pack_started = None
        pack_seconds = 0
        pack_workers = 0
        while process.poll() is None:
            try:
                progress = json.loads((root / 'progress.json').read_text())
                maximum_active = max(maximum_active, len(progress.get('activeChunks', [])))
                if progress['stage'] == 'generate-base' and base_started is None:
                    base_started = time.monotonic()
                elif progress['stage'] != 'generate-base' and base_started is not None and base_seconds is None:
                    base_seconds = time.monotonic() - base_started
                if progress['stage'] == 'pack-tiles':
                    if pack_started is None:
                        pack_started = time.monotonic()
                    pack_workers = max(pack_workers, progress.get('packWorkers', 1))
                elif pack_started is not None:
                    pack_seconds += time.monotonic() - pack_started
                    pack_started = None
                if progress['stage'] == 'synchronize-boundaries':
                    if sync_started is None:
                        sync_started = time.monotonic()
                    sync_active = max(sync_active, len(progress.get('activeChunks', [])))
                elif sync_started is not None:
                    sync_seconds += time.monotonic() - sync_started
                    sync_started = None
            except (OSError, ValueError):
                pass
            time.sleep(0.05)
        if pack_started is not None:
            pack_seconds += time.monotonic() - pack_started
        if sync_started is not None:
            sync_seconds += time.monotonic() - sync_started
        if process.returncode:
            raise RuntimeError(f'生成失敗: workers={workers}')
        reports = [json.loads(file.read_text()) for file in root.glob('pack-z*.report.json')]
        assert len(reports) == 9, '保存工程の計測記録が不足'
        pack_seconds = sum(r['elapsedSeconds'] for r in reports)
        pack_workers = max(r['workers'] for r in reports)
        results.append(dict(workers=workers, seconds=time.monotonic() - started, baseSeconds=base_seconds, syncSeconds=sync_seconds, maximumSyncActive=sync_active, packSeconds=pack_seconds, maximumPackWorkers=pack_workers, maximumActive=maximum_active, fingerprint=fingerprint(root)))
    (args.output / 'comparison.json').write_text(json.dumps(results, indent=2) + '\n', encoding='utf-8')
    assert results[0]['fingerprint'] == results[1]['fingerprint'], '並列数で形状・保存BLOBが変わりました'
    assert results[0]['maximumPackWorkers'] == 1
    assert results[1]['maximumPackWorkers'] > 1
    assert results[0]['maximumActive'] == 1
    assert results[1]['maximumActive'] > 1
    assert results[0]['maximumSyncActive'] == 1
    assert results[1]['maximumSyncActive'] > 1
    (args.output / 'comparison.json').write_text(json.dumps(results, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(results, indent=2), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--exe', type=Path, required=True)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--sha256', required=True)
    parser.add_argument('--window', nargs=4, type=int, required=True)
    parser.add_argument('--cells', type=int, default=512)
    main(parser.parse_args())
