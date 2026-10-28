"""評価地点を含む三角形の3頂点だけから平面/生成時の傾き付き曲面を復元する。"""
import lzma
import struct
import numpy as np
from scipy.interpolate import CloughTocher2DInterpolator
from scipy.spatial import Delaunay
from scipy.sparse import csr_matrix
from mls_quant_codec import encode_quant, decode_quant

HEADER = struct.Struct('<4sIIf')
MODE = 'three-vertex-clough-tocher-baked-gradients-v1'


class ThreePointModel:
    def __init__(self, xy, z, gradients=None):
        self.xy = np.ascontiguousarray(xy, dtype=float)
        self.z = np.asarray(z, dtype=float)
        self.gradients = None if gradients is None else np.asarray(gradients, dtype=float)
        self.tri = Delaunay(self.xy)
        if gradients is not None:
            # 勾配の推定を表示時に行わず、生成時に保存した値を明示的に設定する。
            self.curve = CloughTocher2DInterpolator(self.tri, np.zeros(len(xy)))
            self.curve.values[:, 0] = self.z
            self.curve.grad[:, 0, :] = self.gradients

    def __call__(self, q):
        q = np.asarray(q, dtype=float)
        if self.gradients is not None:
            return self.curve(q)
        return self.linear_operator(q)@self.z

    def linear_operator(self, q):
        ids = self.tri.find_simplex(q, tol=1e-10)
        if np.any(ids < 0): raise ValueError('3点曲面の生成領域外です')
        t = self.tri.transform[ids]
        first = np.einsum('bij,bj->bi', t[:, :2], q-t[:, 2])
        weights = np.column_stack((first, 1-first.sum(axis=1)))
        return csr_matrix((weights.ravel(), (np.repeat(np.arange(len(q)), 3),
                           self.tri.simplices[ids].ravel())), shape=(len(q), len(self.xy)))


class CubicHeightBasis:
    """固定した傾きのもとで、高さ3変数に対する係数だけを取り出す。"""
    def __init__(self, xy):
        self.xy = np.asarray(xy, float)
        self.tri = Delaunay(self.xy)
        # 各三角形の3係数を過剰決定の8チャンネルから安定に復元する。
        random_values = np.random.default_rng(99213).normal(size=(len(xy), 8))
        self.sketch = CloughTocher2DInterpolator(self.tri, np.zeros_like(random_values))
        self.sketch.values[:] = random_values
        self.sketch.grad[:] = 0
        self.inverse = np.linalg.pinv(random_values[self.tri.simplices])

    def operator(self, q):
        q = np.asarray(q, float)
        ids = self.tri.find_simplex(q, tol=1e-10)
        if np.any(ids < 0): raise ValueError('3点曲面の生成領域外です')
        values = self.sketch(q)
        weights = np.einsum('bi,bij->bj', values, self.inverse[ids])
        if not np.isfinite(weights).all(): raise ValueError('3点曲面の係数が不正です')
        return csr_matrix((weights.ravel(), (np.repeat(np.arange(len(q)), 3),
                           self.tri.simplices[ids].ravel())), shape=(len(q), len(self.xy)))


def encode(points, gradients, zstep, gradient_step=1/32768):
    geometry = encode_quant(points, xy=.0625, zstep=zstep)
    p = decode_quant(geometry)
    np.testing.assert_array_equal(p[:, :2], np.asarray(points)[:, :2])
    g = np.rint(np.asarray(gradients)/gradient_step)
    if not np.isfinite(g).all() or np.any(abs(g) > np.iinfo(np.int32).max):
        raise ValueError('保存する傾きが範囲外です')
    delta = np.diff(g.astype('<i4'), axis=0, prepend=np.zeros((1, 2), '<i4')).astype('<i4')
    raw = delta.view('u1').reshape(-1, 8).T.copy().tobytes()
    return HEADER.pack(b'CTH1', len(p), len(geometry), gradient_step)+geometry+lzma.compress(raw, preset=6)


def decode(blob):
    if len(blob) < HEADER.size: raise ValueError('CTH1ヘッダーが不足しています')
    magic, count, length, step = HEADER.unpack_from(blob)
    if magic != b'CTH1' or not np.isfinite(step) or step <= 0 or length > len(blob)-HEADER.size:
        raise ValueError('CTH1ヘッダーが不正です')
    points = decode_quant(blob[HEADER.size:HEADER.size+length])
    raw = lzma.decompress(blob[HEADER.size+length:])
    if len(points) != count or len(raw) != count*8: raise ValueError('CTH1の点数が不一致です')
    delta = np.frombuffer(raw, 'u1').reshape(8, count).T.copy().view('<i4').reshape(count, 2)
    gradients = np.cumsum(delta.astype(np.int64), axis=0)*step
    return points, gradients
