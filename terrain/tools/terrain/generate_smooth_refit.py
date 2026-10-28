"""生成時と復元時で同一の滑らかな半径を使い、制御点Zを再適合する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS', '2')
import argparse
import concurrent.futures
import hashlib
import json
import subprocess
import sys
from pathlib import Path
import numpy as np
import mls_lpf_dem as lpf
from mls_dem_levels import write_json
from mls_quant_codec import decode_quant
from mls_smooth_radius import SmoothSupport, model_from_metadata
from raw_dem_reference import RawDem, center_for

ROOT = Path('build/mls-smooth-refit-v1')


def worker(region, errors):
    lpf.ROOT = ROOT
    folder_root = ROOT/region; folder_root.mkdir(parents=True, exist_ok=True)
    raw = RawDem('build/terrain-cache', center_for(region))
    write_json(folder_root/'source-tiles.json', raw.manifest)
    for error in errors:
        kind = f'{error}-lpf-refit'
        folder = folder_root/kind; folder.mkdir(exist_ok=True)
        source = Path('build/mls-lpf-dem-v2')/region/f'{error}-lpf-balanced'
        old_info = json.loads((source/'info.json').read_text())
        if old_info['sourceManifestSha256'] != raw.digest:
            raise ValueError('元DEMが参照元と一致しません')
        source_blob = (source/'model.mlq1').read_bytes()
        xy = decode_quant(source_blob)[:, :2].copy()
        support = SmoothSupport(xy)
        config = dict(region=region, sourceBlobSha256=hashlib.sha256(source_blob).hexdigest(),
                      targetError=error, sigmaMeters=old_info['sigmaMeters'], curvatureFactor=1.,
                      sourceManifestSha256=raw.digest, **support.metadata())
        path = folder/'configuration.json'
        if path.exists() and json.loads(path.read_text()) != config:
            raise ValueError('生成条件が以前と異なります。別の出力先が必要です')
        write_json(path, config)
        if (lpf.VIEW/region/f'{kind}.json').exists(): continue
        filtered = lpf.FilteredDem(region, config['sigmaMeters'])
        if filtered.digest != old_info['filterHaloManifestSha256']:
            raise ValueError('LPF用の周辺元DEMが以前と一致しません')
        if not (folder/'model-info.json').exists():
            lpf.fit(region, error, filtered, xy, True, folder, factor=1., kind=kind, support=support)
        info = json.loads((folder/'model-info.json').read_text())
        info.update(fittingRepeated=True, sameSupportAtFitAndReconstruction=True,
                    additionalDisplaySmoothing=False, xyCoordinatesUnchanged=True,
                    sourceModel=str(source), sourceBlobSha256=config['sourceBlobSha256'])
        write_json(folder/'model-info.json', info)
        p = decode_quant((folder/'model.mlq1').read_bytes())
        np.testing.assert_array_equal(p[:, :2], xy)
        model = model_from_metadata(p, info)
        lpf.verify_export(region, error, kind, raw, filtered, folder, model=model)
        before = json.loads((Path('build/mls-smooth-radius-v1')/region/f'{error}-diagnostics.json').read_text())
        after_path = folder_root/f'{error}-diagnostics.json'
        after = json.loads(after_path.read_text())
        after['same-points-radius-only'] = before[f'{error}-lpf-smooth']
        after['previous-radius'] = before['previous-radius']
        write_json(after_path, after)
    lpf.progress(region, 'all', 'complete', errors=errors)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--region', choices=['fuji', 'yari', 'tsukuba'])
    parser.add_argument('--errors', type=int, nargs='+', default=[50])
    args = parser.parse_args()
    if any(e not in lpf.COUNTS for e in args.errors): raise ValueError('未対応の段階です')
    ROOT.mkdir(exist_ok=True)
    if args.region: return worker(args.region, args.errors)
    write_json(ROOT/'progress.json', dict(stage='generating', errors=args.errors))
    def run(region):
        cmd = [sys.executable, '-X', 'utf8', __file__, '--region', region, '--errors', *map(str, args.errors)]
        with (ROOT/f'{region}.log').open('a', encoding='utf-8') as log:
            proc = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT)
            write_json(ROOT/f'{region}.launch.json', dict(pid=proc.pid, command=cmd))
            if proc.wait(): raise RuntimeError(f'{region}が失敗しました。ログを確認してください')
    with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
        list(pool.map(run, ['fuji', 'yari', 'tsukuba']))
    write_json(ROOT/'progress.json', dict(stage='complete', errors=args.errors))


if __name__ == '__main__': main()
