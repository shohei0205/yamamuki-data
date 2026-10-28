"""保存点を変えずに補間半径だけを滑らかにし、全元画素で再評価する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS', '2')
import argparse
import concurrent.futures
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path
import numpy as np
import mls_lpf_dem as lpf
from mls_dem_levels import write_json
from mls_fast_eval import FastModel
from mls_quant_codec import decode_quant
from mls_smooth_radius import SmoothSupport, model_from_metadata
from raw_dem_reference import RawDem, center_for

ROOT = Path('build/mls-smooth-radius-v1')


def worker(region, errors):
    lpf.ROOT = ROOT
    folder_root = ROOT/region; folder_root.mkdir(parents=True, exist_ok=True)
    raw = RawDem('build/terrain-cache', center_for(region))
    write_json(folder_root/'source-tiles.json', raw.manifest)
    for error in errors:
        kind = f'{error}-lpf-smooth'
        folder = folder_root/kind; folder.mkdir(exist_ok=True)
        old = Path('build/mls-lpf-dem-v2')/region/f'{error}-lpf-balanced'
        old_info = json.loads((old/'info.json').read_text())
        if old_info['sourceManifestSha256'] != raw.digest:
            raise ValueError('元DEMが以前の生成時と一致しません')
        blob = (old/'model.mlq1').read_bytes()
        p = decode_quant(blob)
        support = SmoothSupport(p[:, :2])
        config = dict(sourceBlobSha256=hashlib.sha256(blob).hexdigest(), **support.metadata())
        saved = folder/'configuration.json'
        if saved.exists() and json.loads(saved.read_text()) != config:
            raise ValueError('支持半径の設定または元データが以前と異なります')
        write_json(saved, config)
        if (lpf.VIEW/region/f'{kind}.json').exists(): continue
        shutil.copyfile(old/'model.mlq1', folder/'model.mlq1')
        sigma = old_info['sigmaMeters']
        filtered = lpf.FilteredDem(region, sigma)
        info = {k: v for k, v in old_info.items() if k not in ('artifactDiagnostic', 'filteredTargetValidation',
                                                            'audit', 'validation', 'score')}
        info.update(config, pointCoordinatesUnchanged=True, fittingRepeated=False,
                    sourceModel=str(old), reconstruction='independent-MLS',
                    supportMode=config['supportMode'], sourceFittingAudit=old_info['audit'],
                    sourceFittingValidation=old_info['validation'])
        write_json(folder/'model-info.json', info)
        # 検証と描画が同じ再構築関数を通るよう、今回の方式を渡す。
        model = model_from_metadata(p, info)
        lpf.verify_export(region, error, kind, raw, filtered, folder, model=model)
        old_diagnostic = json.loads((Path('build/mls-lpf-dem-v2')/region/f'{error}-diagnostics.json').read_text())
        new_diagnostic = json.loads((folder_root/f'{error}-diagnostics.json').read_text())
        new_diagnostic['original'] = old_diagnostic['original']
        new_diagnostic['previous-radius'] = old_diagnostic[f'{error}-lpf-balanced']
        write_json(folder_root/f'{error}-diagnostics.json', new_diagnostic)
    lpf.progress(region, 'all', 'complete', errors=errors)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--region', choices=['fuji', 'yari', 'tsukuba'])
    parser.add_argument('--errors', nargs='+', type=int, default=[50])
    args = parser.parse_args()
    if any(e not in lpf.COUNTS for e in args.errors): raise ValueError('未対応の段階です')
    ROOT.mkdir(exist_ok=True)
    if args.region: return worker(args.region, args.errors)
    write_json(ROOT/'progress.json', dict(stage='generating', errors=args.errors))
    def run(region):
        command = [sys.executable, '-X', 'utf8', __file__, '--region', region,
                   '--errors', *map(str, args.errors)]
        with (ROOT/f'{region}.log').open('a', encoding='utf-8') as log:
            process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
            write_json(ROOT/f'{region}.launch.json', dict(pid=process.pid, command=command))
            if process.wait(): raise RuntimeError(f'{region}の処理失敗。ログを確認してください')
    with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
        list(pool.map(run, ['fuji', 'yari', 'tsukuba']))
    write_json(ROOT/'progress.json', dict(stage='complete', errors=args.errors))


if __name__ == '__main__': main()
