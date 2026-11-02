"""実行した試験の結果とバイナリーを固定し、全国実行の準備完了を記録する。"""
from contextlib import closing
import sqlite3
import argparse
import hashlib
import json
from pathlib import Path
import re
from prepare_national_shared import ROOT, save_json


def qualify(prepared, engine, test_log):
    with closing(sqlite3.connect(prepared/'engine-lock.sqlite3', timeout=0)) as guard:
        guard.execute('BEGIN EXCLUSIVE')
        return _qualify(prepared, engine, test_log)


def _qualify(prepared, engine, test_log):
    job = json.loads((prepared/'job.json').read_text(encoding='utf-8'))
    if not job['inputs_verified'] or job['issues']:
        raise ValueError('全国入力の全件検証が必要です')
    log = test_log.read_text(encoding='utf-8-sig')
    results = re.findall(r'test result: ok\. (\d+) passed; 0 failed;', log)
    if not results or int(results[-1]) < 32:
        raise ValueError('現在のRustテスト32件以上の成功が必要です')
    pilots = []
    for region in ['yari', 'fuji', 'tsukuba']:
        root = ROOT/f'build/national-shared-{region}-pilot-v1'
        progress = json.loads((root/'progress.json').read_text(encoding='utf-8'))
        verification = json.loads((root/'verify.json').read_text(encoding='utf-8'))
        if progress.get('complete') is not True or progress.get('national_complete') is not False:
            raise ValueError(f'{region}の試験が未完了です')
        if not verification.get('integers', {}).get('integer_roundtrip') or not verification.get('saved_dem', {}).get('all_valid_pixels'):
            raise ValueError(f'{region}の最終バイナリーでverifyしてください')
        db = root/'terrain-shared.sqlite3'
        pilots.append(dict(region=region, scope='one z14 PNG; all eight output levels', sqlite_bytes=db.stat().st_size,
                           sqlite_sha256=hashlib.sha256(db.read_bytes()).hexdigest(), verification=verification))
    stress = []
    for name in ['yari128', 'tsukuba512']:
        report = json.loads((ROOT/f'build/shared-disk-stress-{name}-v1/report.json').read_text(encoding='utf-8'))
        if not report['complete'] or report['maximum_height_difference'] >= 1e-6:
            raise ValueError('外部メモリー試験が未達です')
        if report['maximum_local_points'] > 131072 or report['resources']['peak_working_set_bytes'] > 8*1024**3:
            raise ValueError('資源上限が未達です')
        stress.append(report)
    proof = dict(format='national-shared-ready-v1', ready_to_generate=True, generation_id=job['generation_id'],
                 engine_sha256=hashlib.sha256(engine.read_bytes()).hexdigest(), engine=str(engine),
                 settings_sha256=job['identity']['settings_sha256'], rust_tests=int(results[-1]),
                 pilots=pilots, stress=stress,
                 limitations=['全国DEM全体の生成完了ではない', '512km試験は既存実地形制御点の独立複製であり、新規DEM適合ではない',
                              '全国実行中も資源・支持点上限・全画素誤差・最小角・復号を監視し、未達時は停止する'])
    save_json(prepared/'ready.json', proof)
    job['ready_to_generate'] = True
    job['blockers'] = []
    job['engine_preflight'] = dict(path='ready.json', engine_sha256=proof['engine_sha256'])
    save_json(prepared/'job.json', job)
    return proof


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--prepared', type=Path, default=ROOT/'build/national-shared-z6-z13-v1')
    p.add_argument('--engine', type=Path, default=ROOT/'build/natural-fixed-lpf-target/release/terrain-natural-shared-national.exe')
    p.add_argument('--test-log', type=Path, required=True)
    a = p.parse_args()
    result = qualify(a.prepared.resolve(), a.engine.resolve(), a.test_log.resolve())
    print(json.dumps({k: result[k] for k in ['ready_to_generate', 'generation_id', 'engine_sha256', 'rust_tests']}, ensure_ascii=False, indent=2))
