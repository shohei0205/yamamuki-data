"""近傍選択は従来どおり、二次MLSの小行列計算だけをネイティブで行う。"""
import ctypes
from pathlib import Path
import numpy as np
from mls_generated_stable import DirectSupport


class FastModel:
    def __init__(self, xy, z, support=None):
        self.xy = np.ascontiguousarray(xy, dtype=np.float64)
        self.z = np.ascontiguousarray(z, dtype=np.float64)
        self.support = DirectSupport(self.xy) if support is None else support
        if self.support.tree.n != len(self.xy) or not np.array_equal(self.support.tree.data, self.xy):
            raise ValueError('支持半径の近傍木とXY座標が一致しません')
        self.lib = ctypes.CDLL(str(Path('build/mls_eval.dll').resolve()))
        array = np.ctypeslib.ndpointer(dtype=np.float64, flags='C_CONTIGUOUS')
        ints = np.ctypeslib.ndpointer(dtype=np.int64, flags='C_CONTIGUOUS')
        self.lib.mls_eval.argtypes = [array, array, array, array, ints, ints, ctypes.c_int, array]
        self.lib.mls_eval.restype = ctypes.c_int

    def __call__(self, query):
        query = np.ascontiguousarray(query, dtype=np.float64)
        out = np.empty(len(query))
        for start in range(0, len(query), 8192):
            q = query[start:start+8192]
            radius = np.ascontiguousarray(self.support(q))
            groups = self.support.tree.query_ball_point(q, radius, return_sorted=True)
            lengths = np.fromiter(map(len, groups), dtype=np.int64)
            if lengths.min() < 6:
                raise ValueError('MLSの近傍点が6点未満です')
            offsets = np.r_[np.int64(0), np.cumsum(lengths)]
            ids = np.concatenate(groups).astype(np.int64)
            code = self.lib.mls_eval(self.xy, self.z, q, radius, offsets, ids,
                                     len(q), out[start:start+len(q)])
            if code:
                raise ValueError(f'MLS行列が不正: {start+code-1}')
        return out
