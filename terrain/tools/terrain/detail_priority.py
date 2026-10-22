"""共通プールの追加点を、平坦部より山の形へ配分する重み。"""
import numpy as np
from scipy.ndimage import gaussian_filter, maximum_filter, minimum_filter


def priority_gain(grid, landform, valid, spacing=31.25):
    # 約125 mで地形をならし、微小な起伏やDEMのノイズを勾配と誤認しない。
    smooth = gaussian_filter(np.asarray(grid, dtype=np.float32), 125. / spacing)
    gy, gx = np.gradient(smooth, spacing)
    slope = np.hypot(gx, gy)
    # 山頂そのものは勾配ゼロになるため、約250 m周囲の斜面も参照する。
    slope = maximum_filter(slope, size=2 * round(250. / spacing) + 1)
    low, high = np.tan(np.deg2rad([1., 8.]))
    t = np.clip((slope - low) / (high - low), 0., 1.)
    gate = .02 + .98 * t * t * (3. - 2. * t)
    gain = gate * (1. + .5 * np.clip(landform, 0., 1.))
    gain[~valid] = .02
    return gain.astype(np.float32)


def shape_priority_gain(grid, landform, valid, spacing=31.25):
    gain=priority_gain(grid,landform,valid,spacing)
    smooth=gaussian_filter(np.asarray(grid,dtype=np.float32),125./spacing)
    size=2*round(2000./spacing)+1
    relief=maximum_filter(smooth,size=size)-minimum_filter(smooth,size=size)
    # 半径2 kmの比高が小さい山では、同じ標高差を最大4倍に評価する。
    relative=np.clip(800./np.maximum(relief,200.),1.,4.)
    gain*=relative
    gain[~valid]=.02
    return gain
