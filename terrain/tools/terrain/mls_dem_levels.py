"""元PNG DEM基準で独立25 m・独立12.5 m・残差12.5 mを比較する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS', '4')
import gc
import hashlib
import json
import shutil
import time
from pathlib import Path
import numpy as np
from scipy.sparse.linalg import lsmr
from scipy.stats import qmc
from free_mls import operator
from raw_dem_reference import RawDem, center_for, SPAN
from mls_dem_independent import native_sample
from mls_generated_stable import DirectSupport, generate_linear
from mls_high_precision import evaluate
from mls_quant_codec import encode_quant, decode_quant
from mls_residual_curve import encode as encode_residual, decode as decode_residual
from mls_error_quality import requirements, peak_samples

ROOT = Path('build/mls-dem-levels-yari-v1')
VIEW = Path('build/mls-generated-preview/yari-dem')


def write_json(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2), encoding='utf-8', newline='\n')


def progress(kind, stage, **values):
    value = dict(model=kind, stage=stage, **values)
    write_json(ROOT / 'progress.json', value)
    print(json.dumps(value, ensure_ascii=False), flush=True)


def predictor(points):
    support = DirectSupport(points[:, :2])
    return lambda q: evaluate(points[:, :2], points[:, 2], q, support)


def fit(source, xy, error, folder, kind, parent=None):
    support = DirectSupport(xy)
    native, z_native = native_sample(source, 8)
    extra = np.vstack((xy, (qmc.Sobol(2, scramble=True, seed=29004).random_base2(15)-.5)*SPAN,
                       peak_samples(14, 29452, 1500)))
    target_extra = source.sample(extra)
    valid = np.isfinite(target_extra)
    q = np.vstack((native, extra[valid]))
    truth = np.r_[z_native, target_extra[valid]]
    limits = np.where(np.linalg.norm(q, axis=1) <= 2000, error/2, error)
    progress(kind, 'fit-operator', controls=len(xy), samples=len(q))
    a = operator(xy, q, support, curvature=1e-5, tree=support.tree)
    parent_predict = predictor(parent) if parent is not None else None
    fixed = parent_predict(q) if parent is not None else np.zeros(len(q))
    target = truth-fixed
    z = np.zeros(len(xy)) if parent is not None else source.sample(xy)
    weights = 1/limits
    best = (float(np.max(abs(a@z-target)/limits)), z.copy())
    started = time.perf_counter()
    for iteration in range(8):
        z += lsmr(a.multiply(weights[:, None]), (target-a@z)*weights,
                  damp=.003, maxiter=500, atol=1e-7, btol=1e-7)[0]
        delta = a@z-target
        score = float(np.max(abs(delta)/limits))
        if score < best[0]:
            best = score, z.copy()
            np.save(folder/'coefficients.npy', z)
        progress(kind, 'fit', iteration=iteration, normalizedError=score,
                 bestNormalizedError=best[0], seconds=time.perf_counter()-started)
        if score < .8:
            break
        weights = np.maximum(weights, np.clip(abs(delta)/limits, 1, 24)**2/limits)
    z = best[1]
    np.save(folder/'coefficients.npy', z)
    validation_q = np.vstack(((qmc.Sobol(2, scramble=True, seed=29904).random_base2(15)-.5)*SPAN,
                               peak_samples(14, 29935, 1000)))
    validation_truth = source.sample(validation_q)
    av = operator(xy, validation_q, support, curvature=1e-5, tree=support.tree)
    pv = parent_predict(validation_q) if parent is not None else np.zeros(len(validation_q))
    choices = []
    for step in (.125, .5, 1, 2, 4):
        if parent is None:
            blob = encode_quant(np.column_stack((xy, z)), xy=.0625, zstep=step)
            points = decode_quant(blob)
            np.testing.assert_array_equal(points[:, :2], xy)
            restored = points[:, 2]
        else:
            blob = encode_residual(parent, z, step)
            positions, restored = decode_residual(parent, blob)
            np.testing.assert_array_equal(positions, xy)
        audit = requirements(fixed+a@restored-truth, q, error)
        validation = requirements(pv+av@restored-validation_truth, validation_q, error)
        score = max(audit['measuredMaxError']/error, audit['measuredPeakMaxError']/(error/2),
                    validation['measuredMaxError']/error, validation['measuredPeakMaxError']/(error/2))
        passed = audit['converged'] and validation['converged']
        item = dict(step=step, bytes=len(blob), audit=audit, validation=validation, score=score)
        choices.append(((not passed, len(blob) if passed else score), item, blob, restored))
    _, info, blob, z = min(choices, key=lambda value: value[0])
    write_json(folder/'trials.json', [c[1] for c in choices])
    (folder/('model.mlq1' if parent is None else 'residual.mlr1')).write_bytes(blob)
    del a, av, choices
    gc.collect()
    info.update(targetError=error, reference='original-PNG-DEM', sourceManifestSha256=source.digest,
                controls=len(xy)+(0 if parent is None else len(parent)),
                residualControls=0 if parent is None else len(xy),
                parentControls=0 if parent is None else len(parent),
                reconstruction='independent-MLS' if parent is None else 'parent-plus-residual-MLS',
                parentBytes=0 if parent is None else (ROOT/'25-independent/model.mlq1').stat().st_size)
    info['totalBytes'] = info['bytes']+info['parentBytes']
    if parent is not None:
        info['parentSha256'] = hashlib.sha256((ROOT/'25-independent/model.mlq1').read_bytes()).hexdigest()
    write_json(folder/'model-info.json', info)
    return xy, z, support, info


def verify_and_preview(source, kind, predict, info):
    folder = ROOT/kind
    maximum = peak_max = squared = 0.
    count = missing = 0
    worst = None
    started = time.perf_counter()
    for block, (q, truth, invalid) in enumerate(source.native_rows(16)):
        delta = predict(q)-truth
        i = int(np.argmax(abs(delta)))
        if abs(delta[i]) > maximum:
            maximum = float(abs(delta[i]))
            worst = dict(xy=q[i].tolist(), dem=float(truth[i]), model=float(truth[i]+delta[i]))
        peak = np.linalg.norm(q, axis=1) <= 2000
        if peak.any():
            peak_max = max(peak_max, float(abs(delta[peak]).max()))
        squared += float(delta@delta)
        count += len(delta)
        missing += invalid
        if block % 8 == 0:
            progress(kind, 'native-DEM-verification', samples=count, maxError=maximum,
                     rmse=(squared/count)**.5, seconds=time.perf_counter()-started)
    error = info['targetError']
    info.update(measuredMaxError=maximum, measuredPeakMaxError=peak_max,
                measuredRmse=(squared/count)**.5, samples=count, missing=missing, worst=worst,
                converged=maximum <= error and peak_max <= error/2, n=1024, span=SPAN)
    write_json(folder/'info.json', info)
    export_preview(kind, predict, info)
    return info


def export_preview(kind, predict, info):
    n = 1024
    for span in (32000, 8000):
        progress(kind, 'preview', span=span)
        axis = np.linspace(-span/2, span/2, n+1)
        x, y = np.meshgrid(axis, axis)
        q = np.column_stack((x.ravel(), y.ravel()))
        height = predict(q).reshape(n+1, n+1)
        gy, gx = np.gradient(height, span/n, edge_order=2)
        np.stack((height, gx, gy), axis=2).astype('<f4').tofile(VIEW/f'{kind}-{span}.bin')
    write_json(VIEW/f'{kind}.json', info)


def main():
    ROOT.mkdir(exist_ok=True)
    VIEW.mkdir(exist_ok=True)
    source = RawDem('build/terrain-cache', center_for('yari'))
    manifest = ROOT/'source-tiles.json'
    if manifest.exists() and json.loads(manifest.read_text()) != source.manifest:
        raise ValueError('元DEMが前回と一致しません')
    write_json(manifest, source.manifest)
    kind = '25-independent'
    folder = ROOT/kind
    folder.mkdir(exist_ok=True)
    if not (folder/'info.json').exists():
        if (folder/'model-info.json').exists():
            p = decode_quant((folder/'model.mlq1').read_bytes())
            info = json.loads((folder/'model-info.json').read_text())
        else:
            old = np.fromfile('build/mls-residual-curve-yari-v2/parent.f32', '<f4').reshape(-1, 3)
            xy = decode_quant(encode_quant(np.column_stack((old[:, :2], np.zeros(len(old)))), xy=.0625))[:, :2].copy()
            xy, z, support, info = fit(source, xy, 25, folder, kind)
            p = decode_quant((folder/'model.mlq1').read_bytes())
        verify_and_preview(source, kind, predictor(p), info)
    parent = decode_quant((folder/'model.mlq1').read_bytes())
    if not (VIEW/f'{kind}.json').exists():
        export_preview(kind, predictor(parent), json.loads((folder/'info.json').read_text()))
    kind = '12.5-independent'
    folder = ROOT/kind
    folder.mkdir(exist_ok=True)
    if not (folder/'info.json').exists():
        # 前回の元DEM適合済み・全元画素検証済みデータを同じ基準の結果として再利用する。
        old = Path('build/mls-dem-independent-yari-v1')
        info = json.loads((old/'info.json').read_text())
        if info['reference'] != 'original-PNG-DEM' or info['sourceManifestSha256'] != source.digest:
            raise ValueError('独立12.5 mの評価基準が異なります')
        info.update(info['nativeDemValidation'], totalBytes=info['bytes'], parentBytes=0, n=1024, span=SPAN,
                    reusedFrom=str(old))
        shutil.copyfile(old/'model.mlq1', folder/'model.mlq1')
        write_json(folder/'info.json', info)
        for span in (32000, 8000):
            shutil.copyfile(VIEW/f'model-{span}.bin', VIEW/f'{kind}-{span}.bin')
        write_json(VIEW/f'{kind}.json', info)
    if not (VIEW/f'{kind}.json').exists():
        p = decode_quant((folder/'model.mlq1').read_bytes())
        export_preview(kind, predictor(p), json.loads((folder/'info.json').read_text()))
    kind = '12.5-residual'
    folder = ROOT/kind
    folder.mkdir(exist_ok=True)
    if not (folder/'info.json').exists():
        if (folder/'model-info.json').exists():
            xy, z = decode_residual(parent, (folder/'residual.mlr1').read_bytes())
            support = DirectSupport(xy)
            info = json.loads((folder/'model-info.json').read_text())
        else:
            xy = generate_linear(parent, 1)[:, :2].copy()
            xy, z, support, info = fit(source, xy, 12.5, folder, kind, parent)
        parent_predict = predictor(parent)
        predict = lambda q: parent_predict(q)+evaluate(xy, z, q, support)
        verify_and_preview(source, kind, predict, info)
    if not (VIEW/f'{kind}.json').exists():
        xy, z = decode_residual(parent, (folder/'residual.mlr1').read_bytes())
        support = DirectSupport(xy)
        parent_predict = predictor(parent)
        export_preview(kind, lambda q: parent_predict(q)+evaluate(xy, z, q, support),
                       json.loads((folder/'info.json').read_text()))
    reports = {kind: json.loads((ROOT/kind/'info.json').read_text())
               for kind in ('25-independent', '12.5-independent', '12.5-residual')}
    write_json(ROOT/'summary.json', reports)
    progress('all', 'complete', results=reports)


if __name__ == '__main__':
    main()
