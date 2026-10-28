"""6点程度の狭い二次MLSで生成時から適合し、従来版と比較する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS', '2')
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
from mls_smooth_radius import SixPointSupport, model_from_metadata
from raw_dem_reference import RawDem, center_for

ROOT = Path('build/mls-six-point-v3')


def worker(region):
    lpf.ROOT = ROOT; (ROOT/region).mkdir(parents=True, exist_ok=True)
    source = Path('build/mls-smooth-refit-v1')/region/'50-lpf-refit'
    old = json.loads((source/'info.json').read_text())
    blob = (source/'model.mlq1').read_bytes(); xy = decode_quant(blob)[:, :2].copy()
    raw = RawDem('build/terrain-cache', center_for(region))
    filtered = lpf.FilteredDem(region, old['sigmaMeters'])
    if raw.digest != old['sourceManifestSha256'] or filtered.digest != old['filterHaloManifestSha256']:
        raise ValueError('元DEMが参照元と一致しません')
    kind = '50-six-trained'; folder = ROOT/region/kind; folder.mkdir(exist_ok=True)
    support = SixPointSupport(xy)
    config = dict(sourceBlobSha256=hashlib.sha256(blob).hexdigest(), generationSlopeWeight=2., **support.metadata())
    path = folder/'configuration.json'
    if path.exists() and json.loads(path.read_text()) != config: raise ValueError('生成条件が一致しません')
    write_json(path, config)
    write_json(ROOT/region/'source-tiles.json', raw.manifest)
    if (lpf.VIEW/region/f'{kind}.json').exists(): return
    if not (folder/'model-info.json').exists():
        lpf.fit(region, 50, filtered, xy, True, folder, factor=1., kind=kind, support=support,
                slope_weight=2.)
    info = json.loads((folder/'model-info.json').read_text())
    info.update(fittingRepeated=True, sameSupportAtFitAndReconstruction=True,
                additionalDisplaySmoothing=False, xyCoordinatesUnchanged=True,
                sourceModel=str(source), sourceBlobSha256=config['sourceBlobSha256'])
    points = decode_quant((folder/'model.mlq1').read_bytes())
    np.testing.assert_array_equal(points[:, :2], xy)
    model = model_from_metadata(points, info)
    q = np.random.default_rng(990).uniform(-16000, 16000, (32768, 2))
    radius = model.support(q)
    counts = model.support.tree.query_ball_point(q, radius, return_length=True)
    if counts.min() < 6: raise ValueError('6点未満の補間範囲が見つかりました')
    info.update(neighborCountMin=int(counts.min()), neighborCountMedian=float(np.median(counts)),
                neighborCountMax=int(counts.max()), radiusMetersMedian=float(np.median(radius)))
    write_json(folder/'model-info.json', info)
    lpf.verify_export(region, 50, kind, raw, filtered, folder, model=model)
    diag_path = ROOT/region/'50-diagnostics.json'
    diag = json.loads(diag_path.read_text())
    prev = json.loads((Path('build/mls-smooth-refit-v1')/region/'50-diagnostics.json').read_text())
    diag['wide-refit'] = prev['50-lpf-refit']; write_json(diag_path, diag)
    lpf.progress(region, kind, 'complete')


def main():
    ROOT.mkdir(exist_ok=True)
    if len(sys.argv) > 1: return worker(sys.argv[1])
    write_json(ROOT/'progress.json', dict(stage='generating'))
    def run(region):
        with (ROOT/f'{region}.log').open('a', encoding='utf-8') as log:
            proc = subprocess.Popen([sys.executable, '-X', 'utf8', __file__, region], stdout=log, stderr=subprocess.STDOUT)
            write_json(ROOT/f'{region}.launch.json', dict(pid=proc.pid))
            if proc.wait(): raise RuntimeError(f'{region}で失敗。ログを確認してください')
    with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
        list(pool.map(run, ['fuji', 'yari', 'tsukuba']))
    write_json(ROOT/'progress.json', dict(stage='complete', regions=3, error=50))


if __name__ == '__main__': main()
