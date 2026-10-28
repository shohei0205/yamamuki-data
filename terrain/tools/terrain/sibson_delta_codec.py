"""親の自然近傍曲面から整数 Z を予測する可逆差分。"""
import hashlib
import sqlite3
import struct
import time
import numpy as np
from mls_quant_codec import encode_quant, decode_quant
from sibson_surface import SibsonSurface

HEADER = struct.Struct('<4sd')
UNIT = .125


def checksum(points):
    return hashlib.sha256(np.asarray(points, dtype='<f4').tobytes()).digest()


def encode_delta(points, parent, parent_error):
    if not np.isfinite(parent_error) or parent_error <= 0:
        raise ValueError('親の許容誤差が不正です')
    p = np.asarray(points, dtype=np.float64)
    predicted = np.rint(parent(p[:, :2])/UNIT)
    units = np.rint(p[:, 2]/UNIT)
    if not np.allclose(units*UNIT, p[:, 2], atol=1e-8, rtol=0):
        raise ValueError('元の Z が 0.125 m 刻みではありません')
    residual = np.column_stack((p[:, :2], (units-predicted)*UNIT))
    return HEADER.pack(b'SDZ1', parent_error)+encode_quant(residual, xy=.0625, zstep=UNIT)


def decode_delta(blob, parent, expected_parent):
    if len(blob) <= HEADER.size:
        raise ValueError('差分ヘッダーが不足しています')
    magic, parent_error = HEADER.unpack_from(blob)
    if magic != b'SDZ1' or parent_error != expected_parent:
        raise ValueError('差分の親が一致しません')
    p = decode_quant(blob[HEADER.size:])
    predicted = np.rint(parent(p[:, :2])/UNIT)
    p[:, 2] = (predicted+np.rint(p[:, 2]/UNIT))*UNIT
    return p


def load_level(db, region, error, build_final=True):
    """キャッシュなしの読込み。参照 BLOB・予測・曲面構築を別計測する。"""
    stats = dict(bytes=0, levels=[], blobDecodeSeconds=0., modelBuildSeconds=0.,
                 predictionSeconds=0.)
    cache = {}
    started = time.perf_counter()

    def load(e, stack):
        if e in stack or len(stack) >= 16:
            raise ValueError('親の参照が循環、または深すぎます')
        if e in cache:
            return cache[e]
        row = db.execute('SELECT parent_error,blob,sha256 FROM levels WHERE region=? AND error=?',
                         (region, e)).fetchone()
        if row is None:
            raise ValueError('必要な親レベルがありません')
        parent_error, blob, digest = row
        stats['bytes'] += len(blob); stats['levels'].append(e)
        if parent_error is None:
            t = time.perf_counter(); p = decode_quant(blob)
            stats['blobDecodeSeconds'] += time.perf_counter()-t
        else:
            if parent_error <= e:
                raise ValueError('親は粗いレベルにしてください')
            _, parent_model = load(parent_error, stack+[e])
            if len(blob) <= HEADER.size or HEADER.unpack_from(blob) != (b'SDZ1', parent_error):
                raise ValueError('差分の親が一致しません')
            t = time.perf_counter(); p = decode_quant(blob[HEADER.size:])
            stats['blobDecodeSeconds'] += time.perf_counter()-t
            t = time.perf_counter(); prediction = np.rint(parent_model(p[:, :2])/UNIT)
            p[:, 2] = (prediction+np.rint(p[:, 2]/UNIT))*UNIT
            stats['predictionSeconds'] += time.perf_counter()-t
        if checksum(p) != digest:
            raise ValueError('復元点の検査値が一致しません。親または予測器が不適合です')
        model = None
        if build_final or e != error:
            t = time.perf_counter(); model = SibsonSurface(p)
            stats['modelBuildSeconds'] += time.perf_counter()-t
        cache[e] = (p, model)
        return p, model

    p, model = load(error, [])
    stats['seconds'] = time.perf_counter()-started
    return p, model, stats


def create_database(path):
    if path.exists():
        raise ValueError('比較結果を上書きしないよう、新しい出力先を指定してください')
    db = sqlite3.connect(path)
    db.execute('CREATE TABLE levels(region TEXT,error REAL,parent_error REAL,blob BLOB,sha256 BLOB,PRIMARY KEY(region,error)) WITHOUT ROWID')
    db.execute('CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT) WITHOUT ROWID')
    db.executemany('INSERT INTO metadata VALUES(?,?)', [
        ('format', 'Sibson SDZ1 experiment v1'), ('xyUnitMeters', '0.0625'),
        ('zUnitMeters', '0.125'), ('predictor', 'sibson_eval.c double; nearest integer ties-to-even'),
        ('validation', 'SHA-256 of restored little-endian float32 XYZ')])
    return db
