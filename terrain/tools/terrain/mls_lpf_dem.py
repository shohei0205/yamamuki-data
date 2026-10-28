"""LPF目標と曲率抑制を、同じ自由配置点の既存MLSと比較する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS', '2')
import argparse
import concurrent.futures
import gc
import json
import subprocess
import sys
import time
from pathlib import Path
import numpy as np
from scipy.ndimage import gaussian_filter
from scipy.sparse import csr_matrix, vstack
from scipy.sparse.linalg import lsmr
from scipy.stats import qmc
from free_mls import operator
from mls_dem_independent import native_sample
from mls_dem_regions import COUNTS, canonical, template
from mls_dem_levels import write_json
from mls_error_quality import requirements, peak_samples
from mls_fast_eval import FastModel
from mls_generated_stable import DirectSupport
from mls_quant_codec import encode_quant, decode_quant
from raw_dem_reference import RawDem, center_for, SPAN, STEP

ROOT = Path('build/mls-lpf-dem-v1')
VIEW = Path('build/mls-generated-preview/dem-regions')


def normalized_lpf(grid, sigma_pixels):
    valid = np.isfinite(grid)
    weights = gaussian_filter(valid.astype(float), sigma_pixels, truncate=4, mode='nearest')
    numerator = gaussian_filter(np.where(valid, grid, 0).astype(float), sigma_pixels,
                                truncate=4, mode='nearest')
    return np.where(valid & (weights > 1e-12), numerator/np.maximum(weights, 1e-12), np.nan)


class FilteredDem(RawDem):
    def __init__(self, region, sigma_meters):
        if not np.isfinite(sigma_meters) or sigma_meters <= 0:
            raise ValueError('LPF幅は正の有限値にしてください')
        super().__init__('build/terrain-cache', center_for(region), half=SPAN/2+4*sigma_meters+2*STEP)
        self.grid = normalized_lpf(self.grid, sigma_meters/STEP)
        self.sigma_meters = sigma_meters


def curvature_matrix(xy):
    """近傍の一次面からのずれを測る。傾斜した平面には罰則を与えない。"""
    from scipy.spatial import cKDTree
    tree = cKDTree(xy)
    distance, ids = tree.query(xy, k=min(13, len(xy)))
    distance, ids = distance[:, 1:], ids[:, 1:]
    local = (xy[ids]-xy[:, None, :])/np.maximum(distance[:, -1, None, None], 1e-9)
    p = np.concatenate((np.ones((*local.shape[:2], 1)), local), axis=2)
    weights = 1/np.maximum(distance/distance[:, -1, None], .1)**2
    matrix = np.einsum('bki,bk,bkj->bij', p, weights, p)
    matrix += np.eye(3)*1e-12
    rhs = np.zeros((len(xy), 3, 1)); rhs[:, 0, 0] = 1
    alpha = np.linalg.solve(matrix, rhs)[:, :, 0]
    w = weights*np.einsum('bki,bi->bk', p, alpha)
    rows = np.repeat(np.arange(len(xy)), ids.shape[1])
    smooth = csr_matrix((-w.ravel(), (rows, ids.ravel())), shape=(len(xy), len(xy)))
    return smooth+csr_matrix((np.ones(len(xy)), (np.arange(len(xy)), np.arange(len(xy)))), shape=smooth.shape)


def progress(region, kind, stage, **values):
    report = dict(region=region, model=kind, stage=stage, **values)
    write_json(ROOT/region/'progress.json', report)
    print(json.dumps(report, ensure_ascii=False), flush=True)


def fit(region, error, filtered, xy, stable, folder, factor=5., kind=None, support=None,
        slope_weight=0.):
    kind = kind or f'{error}-'+('lpf-stable' if stable else 'lpf')
    native, native_z = native_sample(filtered, 8)
    extra = np.vstack((xy, (qmc.Sobol(2, scramble=True, seed=29004).random_base2(15)-.5)*SPAN,
                       peak_samples(14, 29452, 1500)))
    truth_extra = filtered.sample(extra)
    valid = np.isfinite(truth_extra)
    q = np.vstack((native, extra[valid])); truth = np.r_[native_z, truth_extra[valid]]
    support = DirectSupport(xy) if support is None else support
    progress(region, kind, 'operator', controls=len(xy), sigma=filtered.sigma_meters)
    a = operator(xy, q, support, curvature=1e-5, tree=support.tree)
    initial = filtered.sample(xy)
    z = initial.copy()
    limits = np.where(np.linalg.norm(q, axis=1) <= 2000, error/2, error)
    weights = 1/limits
    best = float(np.max(abs(a@z-truth)/limits)), z.copy()
    penalty = curvature_matrix(xy)*(factor/error) if stable else None
    slope_design = slope_rhs = None
    if slope_weight:
        # 表示時の平滑化ではなく、狭い補間で得られる傾きを生成時の目的に入れる。
        gq = np.vstack(((qmc.Sobol(2, scramble=True, seed=772).random_base2(15)-.5)*(SPAN-128),
                        peak_samples(13, 882, 1500)))
        delta = 16.
        components = []; targets = []
        for offset in (np.array([delta, 0.]), np.array([0., delta])):
            plus = operator(xy, gq+offset, support, curvature=1e-5, tree=support.tree)
            minus = operator(xy, gq-offset, support, curvature=1e-5, tree=support.tree)
            components.append((plus-minus)/(2*delta))
            targets.append((filtered.sample(gq+offset)-filtered.sample(gq-offset))/(2*delta))
        scale = slope_weight*SPAN/np.sqrt(len(xy))/error
        slope_design = vstack(components).tocsr()*scale
        slope_rhs = np.concatenate(targets)*scale
        height_rms = np.sqrt(np.mean(((a@initial-truth)/limits)**2))
        slope_rms = np.sqrt(np.mean((slope_design@initial-slope_rhs)**2))
        best = float(np.hypot(height_rms, slope_rms)), initial.copy()
    started = time.perf_counter()
    for iteration in range(8):
        if stable:
            # 毎回同じ初期値を中心に解き、反復で補正が蓄積しないようにする。
            blocks = [a.multiply(weights[:, None]), penalty]
            right = [(truth-a@initial)*weights, -penalty@initial]
            if slope_design is not None:
                blocks.append(slope_design); right.append(slope_rhs-slope_design@initial)
            design = vstack(blocks).tocsr()
            rhs = np.concatenate(right)
            z = initial+lsmr(design, rhs, damp=.003, maxiter=500, atol=1e-7, btol=1e-7)[0]
        else:
            z += lsmr(a.multiply(weights[:, None]), (truth-a@z)*weights,
                      damp=.003, maxiter=500, atol=1e-7, btol=1e-7)[0]
        delta = a@z-truth
        height_max = float(np.max(abs(delta)/limits))
        score = height_max if slope_design is None else float(np.hypot(
            np.sqrt(np.mean((delta/limits)**2)), np.sqrt(np.mean((slope_design@z-slope_rhs)**2))))
        if score < best[0]:
            best = score, z.copy()
        progress(region, kind, 'fit', iteration=iteration, bestScore=best[0], seconds=time.perf_counter()-started)
        if height_max < .8 and slope_design is None:
            break
        weights = np.maximum(weights, np.clip(abs(delta)/limits, 1, 8 if stable else 24)**2/limits)
    z = best[1]
    np.save(folder/'coefficients.npy', z)
    v = np.vstack(((qmc.Sobol(2, scramble=True, seed=29904).random_base2(15)-.5)*SPAN,
                   peak_samples(14, 29935, 1000)))
    vt = filtered.sample(v)
    av = operator(xy, v, support, curvature=1e-5, tree=support.tree)
    trials = []
    for step in (.125, .5, 1, 2, 4):
        blob = encode_quant(np.column_stack((xy, z)), xy=.0625, zstep=step)
        p = decode_quant(blob)
        np.testing.assert_array_equal(p[:, :2], xy)
        audit = requirements(a@p[:, 2]-truth, q, error)
        validation = requirements(av@p[:, 2]-vt, v, error)
        score = max(audit['measuredMaxError']/error, audit['measuredPeakMaxError']/(error/2),
                    validation['measuredMaxError']/error, validation['measuredPeakMaxError']/(error/2))
        passed = audit['converged'] and validation['converged']
        info = dict(step=step, bytes=len(blob), audit=audit, validation=validation, score=score)
        if slope_design is not None:
            slope_rms = float(np.sqrt(np.mean((slope_design@p[:, 2]-slope_rhs)**2)))
            slope_unquantized = float(np.sqrt(np.mean((slope_design@z-slope_rhs)**2)))
            combined = float(np.hypot(np.sqrt(np.mean(((a@p[:, 2]-truth)/limits)**2)), slope_rms))
            info.update(normalizedSlopeRms=slope_rms, selectionScore=combined)
            key = (slope_rms > slope_unquantized*1.05+.0005, combined)
        else:
            key = (not passed, len(blob) if passed else score)
        trials.append((key, info, blob))
    _, info, blob = min(trials, key=lambda v: v[0])
    (folder/'model.mlq1').write_bytes(blob)
    write_json(folder/'trials.json', [v[1] for v in trials])
    info.update(region=region, targetError=error, n=1024, span=SPAN, controls=len(xy),
                reconstruction='independent-MLS', reference='original-PNG-DEM',
                fittingReference='Gaussian-filtered-PNG-DEM', sigmaMeters=filtered.sigma_meters,
                filterHaloManifestSha256=filtered.digest, stableFit=stable,
                curvatureWeight=factor/error if stable else 0, curvatureFactor=factor,
                totalBytes=len(blob), parentBytes=0)
    info['generationSlopeWeight'] = slope_weight
    info['selectionObjective'] = 'normalized-height-and-slope-rms' if slope_weight else 'normalized-maximum-height-error'
    if hasattr(support, 'metadata'):
        info.update(support.metadata())
    write_json(folder/'model-info.json', info)
    del a, av, trials
    gc.collect()


def diagnostics(region, error, kind, model, filtered):
    n = 512; step = 8000/n
    axis = np.linspace(-4000, 4000, n+1); x, y = np.meshgrid(axis, axis)
    q = np.column_stack((x.ravel(), y.ravel()))
    target = filtered.sample(q).reshape(n+1, n+1)
    previous = ROOT/region/f'{error}-diagnostics.json'
    reports = json.loads(previous.read_text()) if previous.exists() else {}
    for name, predictor in [(kind, model)]:
        h = predictor(q).reshape(n+1, n+1)
        delta = h-target
        hp = delta-gaussian_filter(delta, 32/step)
        gy, gx = np.gradient(hp, step)
        cut = np.s_[32:-32, 32:-32]
        reports[name] = dict(targetRmse=float(np.sqrt(np.mean(delta[cut]**2))),
                             highFrequencyErrorSlopeRms=float(np.sqrt(np.mean((gx*gx+gy*gy)[cut]))))
    baseline = Path('build/mls-dem-regions-v1')/region/f'{error}-independent/model.mlq1'
    if 'original' not in reports:
        p = decode_quant(baseline.read_bytes())
        baseline_model = FastModel(p[:, :2], p[:, 2])
        delta = baseline_model(q).reshape(n+1, n+1)-target
        hp = delta-gaussian_filter(delta, 32/step); gy, gx = np.gradient(hp, step)
        reports['original'] = dict(targetRmse=float(np.sqrt(np.mean(delta[32:-32, 32:-32]**2))),
                                  highFrequencyErrorSlopeRms=float(np.sqrt(np.mean((gx*gx+gy*gy)[32:-32, 32:-32]))))
    write_json(previous, reports)
    return reports[kind]


def verify_export(region, error, kind, raw, filtered, folder, model=None):
    info = json.loads((folder/'model-info.json').read_text())
    if model is None:
        p = decode_quant((folder/'model.mlq1').read_bytes())
        model = FastModel(p[:, :2], p[:, 2])
    sums = dict(raw=dict(max=0., peak=0., squared=0.), filtered=dict(max=0., peak=0., squared=0.))
    count = missing = 0
    started = time.perf_counter()
    for block, (q, truth, invalid) in enumerate(raw.native_rows(32)):
        actual = model(q)
        target = filtered.sample(q)
        if not np.isfinite(target).all():
            raise ValueError('LPF目標に欠損があります')
        peak = np.linalg.norm(q, axis=1) <= 2000
        for name, reference in [('raw', truth), ('filtered', target)]:
            delta = actual-reference
            s = sums[name]
            s['max'] = max(s['max'], float(abs(delta).max()))
            if peak.any(): s['peak'] = max(s['peak'], float(abs(delta[peak]).max()))
            s['squared'] += float(delta@delta)
        count += len(q); missing += invalid
        if block % 8 == 0:
            progress(region, kind, 'verify', samples=count, seconds=time.perf_counter()-started)
    measures = {}
    for name, s in sums.items():
        measures[name] = dict(measuredMaxError=s['max'], measuredPeakMaxError=s['peak'],
                              measuredRmse=(s['squared']/count)**.5,
                                converged=s['max'] <= error and s['peak'] <= info.get('peakErrorLimit', error/2))
    info.update(measures['raw'], filteredTargetValidation=measures['filtered'], samples=count,
                missing=missing, sourceManifestSha256=raw.digest,
                artifactDiagnostic=diagnostics(region, error, kind, model, filtered))
    write_json(folder/'info.json', info)
    view = VIEW/region
    for span in (32000, 8000):
        progress(region, kind, 'preview', span=span)
        n = info['n']; axis = np.linspace(-span/2, span/2, n+1); x, y = np.meshgrid(axis, axis)
        q = np.column_stack((x.ravel(), y.ravel()))
        for label, h in [(kind, model(q)), (info.get('filteredTargetKey', f'{error}-target'), filtered.sample(q))]:
            height = h.reshape(n+1, n+1); gy, gx = np.gradient(height, span/n, edge_order=2)
            np.stack((height, gx, gy), axis=2).astype('<f4').tofile(view/f'{label}-{span}.bin')
    write_json(view/f'{kind}.json', info)


def worker(region, errors, pilot, factor=5., variant='lpf-stable'):
    (ROOT/region).mkdir(parents=True, exist_ok=True)
    config = dict(region=region, curvatureFactor=factor, variant=variant,
                  filterSigmaRule='0.25*SPAN/sqrt(controlCount)', curvaturePenalty=1e-5)
    config_path = ROOT/region/'configuration.json'
    if config_path.exists() and json.loads(config_path.read_text()) != config:
        raise ValueError('保存済み結果と生成条件が異なります。別の出力先を指定してください')
    write_json(config_path, config)
    raw = RawDem('build/terrain-cache', center_for(region))
    write_json(ROOT/region/'source-tiles.json', raw.manifest)
    for error in errors:
        xy = template(region, COUNTS[error])
        sigma = .25*SPAN/np.sqrt(len(xy))
        filtered = FilteredDem(region, sigma)
        write_json(ROOT/region/f'{error}-halo-tiles.json', filtered.manifest)
        for stable in ([False, True] if pilot else [True]):
            kind = f'{error}-'+(variant if stable else 'lpf')
            folder = ROOT/region/kind; folder.mkdir(exist_ok=True)
            if not (folder/'model-info.json').exists():
                fit(region, error, filtered, xy, stable, folder, factor, kind)
            if not (VIEW/region/f'{kind}.json').exists():
                verify_export(region, error, kind, raw, filtered, folder)
        del filtered
        gc.collect()
    progress(region, 'all', 'complete', errors=errors)


def main():
    global ROOT
    parser = argparse.ArgumentParser()
    parser.add_argument('--region', choices=['fuji', 'yari', 'tsukuba'])
    parser.add_argument('--errors', nargs='+', type=int, default=[50])
    parser.add_argument('--pilot', action='store_true')
    parser.add_argument('--output', type=Path, default=ROOT)
    parser.add_argument('--curvature-factor', type=float, default=5.)
    parser.add_argument('--variant', choices=['lpf-stable', 'lpf-balanced'], default='lpf-stable')
    args = parser.parse_args()
    if any(e not in COUNTS for e in args.errors): raise ValueError('50～3200の対応段階を指定してください')
    if not np.isfinite(args.curvature_factor) or args.curvature_factor <= 0: raise ValueError('曲率抑制係数が不正です')
    ROOT = args.output
    ROOT.mkdir(exist_ok=True)
    if args.region: return worker(args.region, args.errors, args.pilot, args.curvature_factor, args.variant)
    write_json(ROOT/'progress.json', dict(stage='generating', errors=args.errors, pilot=args.pilot,
                                         variant=args.variant, curvatureFactor=args.curvature_factor))
    def run(region):
        cmd = [sys.executable, '-X', 'utf8', __file__, '--region', region, '--errors', *map(str, args.errors)]
        cmd += ['--output', str(ROOT), '--curvature-factor', str(args.curvature_factor), '--variant', args.variant]
        if args.pilot: cmd.append('--pilot')
        with (ROOT/f'{region}.log').open('a', encoding='utf-8') as log:
            process = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT)
            write_json(ROOT/f'{region}.launch.json', dict(pid=process.pid, command=cmd))
            if process.wait(): raise RuntimeError(f'{region}で失敗。ログを確認してください')
    with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
        list(pool.map(run, ['fuji', 'yari', 'tsukuba']))
    write_json(ROOT/'progress.json', dict(stage='complete', errors=args.errors, pilot=args.pilot))


if __name__ == '__main__': main()
