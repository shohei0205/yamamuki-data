"""全国自然近傍地形の入力を固定する。曲面生成や入力取得は行わない。"""
import argparse
from collections import Counter
from contextlib import closing
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import time

ROOT = Path(__file__).resolve().parents[2]
EXPECTED = [(8, 800), (9, 400), (10, 200), (11, 100), (12, 50), (13, 25), (14, 12.5)]


def atomic_json(path, value):
    temporary = path.with_suffix('.next.json')
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n', encoding='utf-8', newline='\n')
    for attempt in range(40):
        try:
            temporary.replace(path)
            return
        except PermissionError:
            if attempt == 39:
                raise
            time.sleep(.05)


def settings(path):
    raw = path.read_bytes()
    config = json.loads(raw)
    if [(level['z'], level['error_m']) for level in config['levels']] != EXPECTED:
        raise ValueError('現在の対象は z8～z14 / 800～12.5 m の7品質です')
    if config['generation']['language'] != 'Rust' or config['codec']['integer_profile'] != 'rice-hybrid-v1':
        raise ValueError('Rust・Rice併用の設定が必要です')
    return config, hashlib.sha256(raw).hexdigest()


def source_tiles(source):
    """索引は必要な入力の一覧。PNGがない行を海として省略しない。"""
    with closing(sqlite3.connect(source.resolve().as_uri() + '?mode=ro', uri=True)) as db:
        previous = None
        for x, y in db.execute('SELECT x,y FROM tiles WHERE z=14 ORDER BY y,x'):
            if not isinstance(x, int) or not isinstance(y, int) or not 0 <= x < 16384 or not 0 <= y < 16384:
                raise ValueError('元DEM索引のタイル番号が不正です')
            if previous == (x, y):
                raise ValueError('元DEM索引に重複タイルがあります')
            previous = (x, y)
            yield x, y


def prepare(source, cache, output, config_path, resume=False):
    source, cache, output, config_path = (Path(p).resolve() for p in (source, cache, output, config_path))
    config, config_hash = settings(config_path)
    if not source.is_file() or not cache.is_dir():
        raise ValueError('元DEM索引SQLiteとPNGキャッシュが必要です')
    # 入力ディレクトリーへ作業DBを混ぜず、既存ファイルも置き換えない。
    if output == cache or cache in output.parents or output == source or output == config_path:
        raise ValueError('出力先は入力とは別の作業ディレクトリーにしてください')
    output.mkdir(parents=True, exist_ok=True)
    with closing(sqlite3.connect(output / 'run-lock.sqlite3', timeout=0)) as guard:
        guard.execute('CREATE TABLE IF NOT EXISTS lock (id INTEGER)')
        guard.commit()
        guard.execute('BEGIN EXCLUSIVE')
        job_path = output / 'job.json'
        identity = dict(config_sha256=config_hash, source_index=str(source), source_cache=str(cache))
        if job_path.exists():
            if not resume:
                raise ValueError('作業が存在します。続行には --resume を指定してください')
            saved = json.loads(job_path.read_text(encoding='utf-8'))
            if saved['identity'] != identity:
                raise ValueError('設定または入力の場所が異なるため、この作業には再開できません')
            job = saved
        else:
            if (output / 'inputs.sqlite3').exists():
                raise ValueError('job.jsonのない入力一覧は上書きしません')
            now = datetime.now(timezone.utc).isoformat()
            job = dict(format='YNN-work-plan-v1', generation_id='YNN-' + now + '-' + config_hash[:12],
                       identity=identity, created_utc=now, complete=False, distributable=False,
                       workers=max(1, (os.cpu_count() or 1) // 2), cache_limit_gib=8,
                       stages=['inventory', 'lpf-audit', 'fit-global-controls', 'quantized-audit',
                               'support-closure', 'pack-rice', 'verify', 'finalize'])
            atomic_json(job_path, job)
            # 設定ハッシュは入力ファイルのバイト列を対象にする。
            (output / 'settings.json').write_bytes(config_path.read_bytes())
        started = time.monotonic()
        progress_path = output / 'progress.json'
        atomic_json(progress_path, dict(stage='inventory', complete=False, done=0, pid=os.getpid()))
        try:
            with closing(sqlite3.connect(output / 'inputs.sqlite3')) as db:
                db.execute('CREATE TABLE IF NOT EXISTS inputs (x INTEGER,y INTEGER,path TEXT,bytes INTEGER,sha256 TEXT,state TEXT,PRIMARY KEY(x,y)) WITHOUT ROWID')
                old = {(x, y): (size, digest, state) for x, y, size, digest, state in db.execute('SELECT x,y,bytes,sha256,state FROM inputs')}
                seen = set()
                digest = hashlib.sha256()
                states = Counter()
                counts = {z: set() for z, _ in EXPECTED}
                for done, (x, y) in enumerate(source_tiles(source), 1):
                    seen.add((x, y))
                    if 'input_manifest_sha256' in job and (x, y) not in old:
                        raise ValueError('固定済み索引にタイルが追加されました。別の出力先を指定してください')
                    path = cache / '14' / str(x) / f'{y}.png'
                    try:
                        # 再開時もハッシュを読み直し、同サイズ・同時刻の変更を見逃さない。
                        data = path.read_bytes()
                        state = 'png-unverified' if data.startswith(b'\x89PNG\r\n\x1a\n') else 'invalid-signature'
                        size, sha = len(data), hashlib.sha256(data).hexdigest()
                    except FileNotFoundError:
                        state, size, sha = 'unavailable-input', 0, None
                    value = (size, sha, state)
                    if (x, y) in old and old[x, y] != value:
                        raise ValueError(f'固定済み入力が変化しました: z14/{x}/{y}。別の出力先を指定してください')
                    db.execute('INSERT OR IGNORE INTO inputs VALUES(?,?,?,?,?,?)', (x, y, str(path), size, sha, state))
                    digest.update(json.dumps([x, y, size, sha, state], separators=(',', ':')).encode() + b'\n')
                    states[state] += 1
                    for z, _ in EXPECTED:
                        counts[z].add((x >> (14-z), y >> (14-z)))
                    if done % 256 == 0:
                        db.commit()
                        atomic_json(progress_path, dict(stage='inventory', done=done, states=dict(states),
                                                       elapsed_seconds=time.monotonic()-started, complete=False, pid=os.getpid()))
                if not seen:
                    raise ValueError('索引にz14の元DEMがありません')
                if set(old) - seen:
                    raise ValueError('固定済み索引からタイルが消えました。別の出力先を指定してください')
                db.commit()
                snapshot = digest.hexdigest()
                if job.get('input_manifest_sha256', snapshot) != snapshot:
                    raise ValueError('元DEM索引が変化しました。別の出力先を指定してください')
                job.update(input_manifest_sha256=snapshot, source_states=dict(states),
                           candidate_tiles={str(z): len(counts[z]) for z, _ in EXPECTED},
                           candidate_counts_are_coverage=False, engine_implemented=False,
                           next_stage='lpf-audit', complete=False, distributable=False)
                atomic_json(job_path, job)
                atomic_json(progress_path, dict(stage='prepared-awaiting-rust-engine', done=len(seen),
                                               states=dict(states), complete=False, distributable=False,
                                               reason='入力一覧のみ。PNG復号・有効画素・coverage・曲面生成は未実施。',
                                               elapsed_seconds=time.monotonic()-started))
                return job
        except Exception as error:
            atomic_json(progress_path, dict(stage='failed', error=str(error), complete=False,
                                           elapsed_seconds=time.monotonic()-started))
            raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, default=ROOT / 'build/terrain-lod.sqlite3')
    parser.add_argument('--cache', type=Path, default=ROOT / 'build/terrain-cache')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--settings', type=Path, default=ROOT / 'NATIONAL_NATURAL_SETTINGS.json')
    parser.add_argument('--resume', action='store_true')
    args = parser.parse_args()
    job = prepare(args.source, args.cache, args.output, args.settings, args.resume)
    print(json.dumps(job, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
