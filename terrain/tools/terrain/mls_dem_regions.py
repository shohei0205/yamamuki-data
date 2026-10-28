"""元PNG DEMから3地域・10段階の独立/残差曲面を生成する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS', '2')
import argparse
import concurrent.futures
import json
import shutil
import sqlite3
import subprocess
import sys
import zlib
from pathlib import Path
import numpy as np
import mls_dem_levels as base
from mls_fast_eval import FastModel
from mls_generated_stable import generate_linear
from mls_quant_codec import encode_quant, decode_quant
from mls_residual_curve import decode as decode_residual
from raw_dem_reference import RawDem, center_for

ROOT = Path('build/mls-dem-regions-v1')
VIEW = Path('build/mls-generated-preview/dem-regions')
COUNTS = {50: 8192, 100: 2048, 200: 512, 400: 128, 800: 64, 1600: 32, 3200: 24}


def canonical(xy):
    return decode_quant(encode_quant(np.column_stack((xy, np.zeros(len(xy)))), xy=.0625))[:, :2].copy()


def template(region, count):
    with sqlite3.connect('build/free-mls-v3/mls.sqlite3') as db:
        blob = db.execute('SELECT points FROM surfaces WHERE region=? AND budget=?',
                          (region, max(128, count))).fetchone()[0]
    p = np.frombuffer(zlib.decompress(blob), '<f4')[1:].reshape(-1, 3)[:, :2].copy()
    corners = np.array([[-16000, -16000], [-16000, 16000], [16000, -16000], [16000, 16000]])
    if count < 128:
        # 疎な品質でも四隅を保持し、内部の候補から距離が離れた点を順次採る。
        selected = list(corners)
        distance = np.min(np.sum((p[:, None, :]-corners[None, :, :])**2, axis=2), axis=1)
        for _ in range(count-4):
            i = int(np.argmax(distance))
            selected.append(p[i].copy())
            distance = np.minimum(distance, np.sum((p-p[i])**2, axis=1))
        p = np.array(selected)
    else:
        used = set()
        for corner in corners:
            i = next(int(i) for i in np.argsort(np.sum((p-corner)**2, axis=1)) if int(i) not in used)
            p[i] = corner
            used.add(i)
    return canonical(p)


def model_from_disk(folder, parent=None):
    if parent is None:
        p = decode_quant((folder/'model.mlq1').read_bytes())
        return FastModel(p[:, :2], p[:, 2])
    xy, z = decode_residual(parent, (folder/'residual.mlr1').read_bytes())
    parent_model = FastModel(parent[:, :2], parent[:, 2])
    residual = FastModel(xy, z)
    return lambda q: parent_model(q)+residual(q)


def worker(region):
    folder_root, view = ROOT/region, VIEW/region
    folder_root.mkdir(parents=True, exist_ok=True)
    view.mkdir(parents=True, exist_ok=True)
    base.ROOT, base.VIEW = folder_root, view
    base.predictor = lambda p: FastModel(p[:, :2], p[:, 2])
    source = RawDem('build/terrain-cache', center_for(region))
    manifest = folder_root/'source-tiles.json'
    if manifest.exists() and json.loads(manifest.read_text()) != source.manifest:
        raise ValueError('元DEMが前回と一致しません')
    base.write_json(manifest, source.manifest)
    # 比較の左側は、同じ元DEMから作った派生描画データ。
    n = 1024
    for span in (32000, 8000):
        dest = view/f'dem-{span}.bin'
        if not dest.exists():
            axis = np.linspace(-span/2, span/2, n+1)
            x, y = np.meshgrid(axis, axis)
            h = source.sample(np.column_stack((x.ravel(), y.ravel()))).reshape(n+1, n+1)
            gy, gx = np.gradient(h, span/n, edge_order=2)
            np.stack((h, gx, gy), axis=2).astype('<f4').tofile(dest)
    # 同じ元PNGで検証した槍ヶ岳の3方式は再計算せず保持する。
    if region == 'yari':
        old = Path('build/mls-dem-levels-yari-v1')
        for kind in ('25-independent', '12.5-independent', '12.5-residual'):
            info = json.loads((old/kind/'info.json').read_text())
            if info['sourceManifestSha256'] != source.digest:
                raise ValueError('再利用する元DEMが不一致')
            dest = folder_root/kind
            if not dest.exists():
                shutil.copytree(old/kind, dest)
            for span in (32000, 8000):
                if not (view/f'{kind}-{span}.bin').exists():
                    shutil.copyfile(Path('build/mls-generated-preview/yari-dem')/f'{kind}-{span}.bin', view/f'{kind}-{span}.bin')
            base.write_json(view/f'{kind}.json', dict(info, region=region))
    parent_xy = canonical(np.vstack((template(region, 8192), generate_linear(
        np.column_stack((template(region, 8192), np.zeros(8192))), 1)[:, :2])))
    levels = [(f'{error}-independent', error, COUNTS[error]) for error in sorted(COUNTS, reverse=True)]
    levels += [('25-independent', 25, None), ('12.5-independent', 12.5, None), ('12.5-residual', 12.5, None)]
    for kind, error, count in levels:
        folder = folder_root/kind
        folder.mkdir(exist_ok=True)
        parent = None
        if kind == '12.5-residual':
            parent = decode_quant((folder_root/'25-independent/model.mlq1').read_bytes())
        if not (folder/'info.json').exists():
            if not (folder/'model-info.json').exists():
                if count:
                    xy = template(region, count)
                elif error == 25:
                    xy = parent_xy
                else:
                    p = decode_quant((folder_root/'25-independent/model.mlq1').read_bytes())
                    added = generate_linear(p, 1)[:, :2].copy()
                    xy = added if parent is not None else canonical(np.vstack((p[:, :2], added)))
                base.fit(source, xy, error, folder, kind, parent)
            info = json.loads((folder/'model-info.json').read_text())
            info.update(region=region, center=source.center.tolist(), pointPlacement='free-XY-regional-template',
                        pointBudget=count, evaluator='native-quadratic-MLS-equivalent')
            base.verify_and_preview(source, kind, model_from_disk(folder, parent), info)
        elif not (view/f'{kind}.json').exists():
            base.export_preview(kind, model_from_disk(folder, parent), json.loads((folder/'info.json').read_text()))
    reports = {kind: json.loads((folder_root/kind/'info.json').read_text()) for kind, _, _ in levels}
    for kind, info in reports.items():
        info['region'] = region
        base.write_json(view/f'{kind}.json', info)
    base.write_json(folder_root/'summary.json', reports)
    base.progress(region, 'complete', levels=len(reports))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--region', choices=['fuji', 'yari', 'tsukuba'])
    args = parser.parse_args()
    if args.region:
        return worker(args.region)
    ROOT.mkdir(exist_ok=True)
    VIEW.mkdir(exist_ok=True)
    def run(region):
        with (ROOT/f'{region}.log').open('a', encoding='utf-8') as log:
            process = subprocess.Popen([sys.executable, '-X', 'utf8', __file__, '--region', region],
                                       stdout=log, stderr=subprocess.STDOUT)
            base.write_json(ROOT/f'{region}.launch.json', dict(pid=process.pid, region=region))
            code = process.wait()
            if code:
                raise RuntimeError(f'{region}で生成失敗。{ROOT/region} とログを確認してください')
    with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
        list(pool.map(run, ['fuji', 'yari', 'tsukuba']))
    reports = {region: json.loads((ROOT/region/'summary.json').read_text()) for region in ['fuji', 'yari', 'tsukuba']}
    base.write_json(ROOT/'summary.json', reports)
    base.write_json(ROOT/'progress.json', dict(stage='complete', regions=3, levels=30))


if __name__ == '__main__':
    main()
