"""表示時は3頂点の局所評価とし、同じ曲面定義で生成時から適合する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS', '2')
import concurrent.futures
import json
import subprocess
import sys
from pathlib import Path
import numpy as np
from scipy.sparse import csr_matrix, vstack
from scipy.sparse.linalg import lsmr
from scipy.stats import qmc
import mls_lpf_dem as lpf
from mls_dem_independent import native_sample
from mls_dem_levels import write_json
from mls_error_quality import peak_samples, requirements
from mls_quant_codec import encode_quant, decode_quant
from raw_dem_reference import RawDem, center_for, STEP
from three_point_surface import ThreePointModel, CubicHeightBasis, encode, decode, MODE

ROOT = Path('build/three-point-surface-v1')


def target_gradients(filtered, xy):
    ex = np.array([STEP, 0.]); ey = ex[::-1]
    return np.column_stack(((filtered.sample(xy+ex)-filtered.sample(xy-ex))/(2*STEP),
                            (filtered.sample(xy+ey)-filtered.sample(xy-ey))/(2*STEP)))


def fit(region, kind, xy, gradients, filtered, folder, cubic):
    native, heights = native_sample(filtered, 8)
    extra = np.vstack((xy, (qmc.Sobol(2, scramble=True, seed=29004).random_base2(15)-.5)*32000,
                       peak_samples(14, 29452, 1500)))
    q = np.vstack((native, extra)); truth = np.r_[heights, filtered.sample(extra)]
    initial = filtered.sample(xy)
    model = ThreePointModel(xy, np.zeros(len(xy)), gradients if cubic else None)
    basis = CubicHeightBasis(xy) if cubic else model
    a = basis.operator(q) if cubic else basis.linear_operator(q)
    fixed = model(q) if cubic else np.zeros(len(q))
    tri = model.tri
    edges = np.unique(np.sort(np.vstack((tri.simplices[:, [0, 1]], tri.simplices[:, [1, 2]],
                                        tri.simplices[:, [2, 0]])), axis=1), axis=0)
    rows = np.repeat(np.arange(len(edges)), 2)
    edge_matrix = csr_matrix((np.tile([-1., 1.], len(edges)), (rows, edges.ravel())),
                             shape=(len(edges), len(xy)))
    edge_target = np.sum((gradients[edges[:, 0]]+gradients[edges[:, 1]])*.5*
                         (xy[edges[:, 1]]-xy[edges[:, 0]]), axis=1)
    limits = np.where(np.linalg.norm(q, axis=1) <= 2000, 25., 50.)
    weights = 1/limits
    best = float(np.max(abs(fixed+a@initial-truth)/limits)), initial.copy()
    for iteration in range(8):
        design = vstack((a.multiply(weights[:, None]), edge_matrix*.01)).tocsr()
        rhs = np.r_[(truth-fixed-a@initial)*weights, (edge_target-edge_matrix@initial)*.01]
        z = initial+lsmr(design, rhs, damp=.003, maxiter=500, atol=1e-7, btol=1e-7)[0]
        error = fixed+a@z-truth; score = float(np.max(abs(error)/limits))
        if score < best[0]: best = score, z.copy()
        lpf.progress(region, kind, 'fit', iteration=iteration, score=score, bestScore=best[0])
        if score < .8: break
        weights = np.maximum(weights, np.clip(abs(error)/limits, 1, 8)**2/limits)
    z = best[1]
    v = np.vstack(((qmc.Sobol(2, scramble=True, seed=29904).random_base2(15)-.5)*32000,
                   peak_samples(14, 29935, 1000)))
    vt = filtered.sample(v); trials = []
    for step in (.125, .5, 1, 2, 4):
        points = np.column_stack((xy, z))
        if cubic:
            blob = encode(points, gradients, step); p, g = decode(blob)
        else:
            blob = encode_quant(points, xy=.0625, zstep=step); p = decode_quant(blob); g = None
        restored = ThreePointModel(p[:, :2], p[:, 2], g)
        audit = requirements(restored(q)-truth, q, 50)
        validation = requirements(restored(v)-vt, v, 50)
        score = max(audit['measuredMaxError']/50, audit['measuredPeakMaxError']/25,
                    validation['measuredMaxError']/50, validation['measuredPeakMaxError']/25)
        passed = audit['converged'] and validation['converged']
        info = dict(step=step, bytes=len(blob), audit=audit, validation=validation, score=score)
        trials.append(((not passed, len(blob) if passed else score), info, blob))
    _, info, blob = min(trials, key=lambda v: v[0])
    (folder/('model.cth1' if cubic else 'model.mlq1')).write_bytes(blob)
    info.update(region=region, targetError=50, n=1024, span=32000., controls=len(xy),
                reconstruction='independent-local-cubic' if cubic else 'independent-local-linear',
                reference='original-PNG-DEM', fittingReference='Gaussian-filtered-PNG-DEM',
                sigmaMeters=filtered.sigma_meters, filterHaloManifestSha256=filtered.digest,
                totalBytes=len(blob), parentBytes=0, sampleVerticesPerEvaluation=3,
                storedGradients=cubic, gradientStep=1/32768 if cubic else None,
                supportMode=MODE if cubic else 'three-vertex-linear-v1',
                sameSupportAtFitAndReconstruction=True, additionalDisplaySmoothing=False,
                geometryTrianglesStored=False)
    write_json(folder/'model-info.json', info)
    write_json(folder/'trials.json', [v[1] for v in trials])


def worker(region):
    lpf.ROOT = ROOT; (ROOT/region).mkdir(parents=True, exist_ok=True)
    source = Path('build/mls-smooth-refit-v1')/region/'50-lpf-refit'
    old = json.loads((source/'info.json').read_text())
    points = decode_quant((source/'model.mlq1').read_bytes()); xy = points[:, :2].copy()
    raw = RawDem('build/terrain-cache', center_for(region))
    if raw.digest != old['sourceManifestSha256']: raise ValueError('元DEMが一致しません')
    filtered = lpf.FilteredDem(region, old['sigmaMeters'])
    if filtered.digest != old['filterHaloManifestSha256']: raise ValueError('LPF用の元DEMが一致しません')
    gradients = target_gradients(filtered, xy)
    write_json(ROOT/region/'source-tiles.json', raw.manifest)
    for cubic in (False, True):
        kind = '50-three-cubic' if cubic else '50-three-linear'
        folder = ROOT/region/kind; folder.mkdir(exist_ok=True)
        if (lpf.VIEW/region/f'{kind}.json').exists(): continue
        if not (folder/'model-info.json').exists(): fit(region, kind, xy, gradients, filtered, folder, cubic)
        info = json.loads((folder/'model-info.json').read_text())
        if cubic:
            p, g = decode((folder/'model.cth1').read_bytes())
        else:
            p = decode_quant((folder/'model.mlq1').read_bytes()); g = None
        model = ThreePointModel(p[:, :2], p[:, 2], g)
        lpf.verify_export(region, 50, kind, raw, filtered, folder, model=model)
    lpf.progress(region, 'all', 'complete')


def main():
    ROOT.mkdir(exist_ok=True)
    if len(sys.argv) > 1: return worker(sys.argv[1])
    write_json(ROOT/'progress.json', dict(stage='generating'))
    def run(region):
        with (ROOT/f'{region}.log').open('a', encoding='utf-8') as log:
            process = subprocess.Popen([sys.executable, '-X', 'utf8', __file__, region], stdout=log, stderr=subprocess.STDOUT)
            write_json(ROOT/f'{region}.launch.json', dict(pid=process.pid))
            if process.wait(): raise RuntimeError(f'{region}が失敗。ログを確認してください')
    with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
        list(pool.map(run, ['fuji', 'yari', 'tsukuba']))
    write_json(ROOT/'progress.json', dict(stage='complete', regions=3, variants=2))


if __name__ == '__main__': main()
