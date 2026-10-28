"""保存XYから連続的な補間半径を復元する。形状の格子は保存しない。"""
import numpy as np
from scipy.ndimage import gaussian_filter, map_coordinates, spline_filter
from mls_generated_stable import DirectSupport
from raw_dem_reference import SPAN

MODE = 'smooth-log-radius-cubic-v1'
SIX_MODE = 'six-neighbor-positive-cubic-radius-v1'


class SixPointSupport:
    """6近傍を必ず内包し、狭い重なりで点の出入りを連続にする。"""
    def __init__(self, xy, intervals=1024, expansion=1.15, halo=4):
        from scipy.spatial import cKDTree
        xy = np.asarray(xy, dtype=float)
        if len(xy) < 6 or xy.ndim != 2 or xy.shape[1] != 2 or not np.isfinite(xy).all():
            raise ValueError('6点支持には有限なXY座標が6点以上必要です')
        if intervals < 16 or expansion <= 1 or halo < 3:
            raise ValueError('6点支持の設定が不正です')
        self.tree = cKDTree(xy)
        self.intervals, self.expansion, self.halo = intervals, expansion, halo
        self.spacing = SPAN/intervals
        axis = (np.arange(-halo, intervals+halo+1)/intervals-.5)*SPAN
        x, y = np.meshgrid(axis, axis)
        q = np.column_stack((x.ravel(), y.ravel()))
        distance = self.tree.query(q, k=6)[0][:, -1].reshape(len(axis), len(axis))
        # d6は1-Lipschitz。正の三次Bスプライン重みの距離二乗平均は
        # 2*h*h/3なので、hの余裕でセル内のd6以上を確保する。
        # 補間係数変換をせず、正の重みで直接平均しC2連続を維持する。
        self.coefficients = (distance+self.spacing)*expansion

    def __call__(self, query):
        q = np.asarray(query, dtype=float)
        if q.ndim != 2 or q.shape[1] != 2 or not np.isfinite(q).all() or np.any(abs(q) > SPAN/2+1e-6):
            raise ValueError('6点支持の地域範囲外または不正な評価座標です')
        coords = (q/SPAN+.5)*self.intervals+self.halo
        return map_coordinates(self.coefficients, coords.T[[1, 0]], order=3,
                               prefilter=False, mode='mirror')

    def metadata(self):
        return dict(supportMode=SIX_MODE, radiusIntervals=self.intervals,
                    radiusExpansion=self.expansion, radiusHaloCells=self.halo,
                    minimumSupportPoints=6, radiusFieldStored=False)


class SmoothSupport:
    def __init__(self, xy, intervals=256, sigma_cells=1., halo=8):
        xy = np.asarray(xy, dtype=float)
        if xy.ndim != 2 or xy.shape[1] != 2 or len(xy) < 24 or not np.isfinite(xy).all():
            raise ValueError('支持半径の復元には有限なXY座標が24点以上必要です')
        if intervals < 16 or sigma_cells <= 0 or halo < int(np.ceil(4*sigma_cells))+2:
            raise ValueError('補間半径の解像度・余白・平滑化幅が不正です')
        direct = DirectSupport(xy)
        self.tree = direct.tree
        self.intervals, self.halo = intervals, halo
        self.spacing = SPAN/intervals
        self.sigma_cells = float(sigma_cells)
        axis = (np.arange(-halo, intervals+halo+1)/intervals-.5)*SPAN
        x, y = np.meshgrid(axis, axis)
        raw = direct(np.column_stack((x.ravel(), y.ravel()))).reshape(len(axis), len(axis))
        log_radius = gaussian_filter(np.log(raw), sigma_cells, truncate=4, mode='mirror')
        self.coefficients = spline_filter(log_radius, order=3, mode='mirror')

    def __call__(self, query):
        q = np.asarray(query, dtype=float)
        if q.ndim != 2 or q.shape[1] != 2 or not np.isfinite(q).all():
            raise ValueError('評価座標は有限なXY座標列にしてください')
        if np.any(abs(q) > SPAN/2+1e-6):
            raise ValueError('滑らかな補間半径の地域範囲外です')
        coords = (q/SPAN+.5)*self.intervals+self.halo
        # 高さを平滑化する処理ではなく、支持半径だけをC2連続に補間する。
        return np.exp(map_coordinates(self.coefficients, coords.T[[1, 0]], order=3,
                                      prefilter=False, mode='mirror'))

    def metadata(self):
        return dict(supportMode=MODE, radiusIntervals=self.intervals,
                    radiusSigmaCells=self.sigma_cells, radiusHaloCells=self.halo,
                    radiusFieldStored=False)


def model_from_metadata(points, info):
    from mls_fast_eval import FastModel
    mode = info.get('supportMode', 'direct')
    if mode == MODE:
        support = SmoothSupport(points[:, :2], info['radiusIntervals'], info['radiusSigmaCells'],
                                info['radiusHaloCells'])
        return FastModel(points[:, :2], points[:, 2], support=support)
    if mode == SIX_MODE:
        support = SixPointSupport(points[:, :2], info['radiusIntervals'], info['radiusExpansion'],
                                  info['radiusHaloCells'])
        return FastModel(points[:, :2], points[:, 2], support=support)
    if mode not in ('direct', 'direct-k24-scale1.5-floor2'):
        raise ValueError(f'未対応の補間半径方式です: {mode}')
    return FastModel(points[:, :2], points[:, 2])
