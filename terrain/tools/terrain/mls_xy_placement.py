"""LPF 目標の曲率と既存曲面の残差から自由 XY を配置する。"""
import json
from pathlib import Path
import numpy as np
from scipy.ndimage import gaussian_filter
from scipy.spatial import cKDTree
from adaptive_allocation_surface import sample_density
from mls_dem_regions import canonical
from mls_quant_codec import decode_quant
from mls_smooth_radius import model_from_metadata


def allocate(filtered, region, count):
    axis = (np.arange(256)+.5)*125-16000
    x, y = np.meshgrid(axis, axis)
    q = np.column_stack((x.ravel(), y.ravel()))
    h = filtered.sample(q).reshape(256,256)
    gy, gx = np.gradient(h,125)
    gyy, gyx = np.gradient(gy,125)
    gxy, gxx = np.gradient(gx,125)
    curvature = np.sqrt(gxx*gxx+gyy*gyy+2*gxy*gxy)
    folder = Path('build/mls-six-point-v3')/region/'50-six-trained'
    info = json.loads((folder/'model-info.json').read_text())
    model = model_from_metadata(decode_quant((folder/'model.mlq1').read_bytes()),info)
    residual = (model(q)-h.ravel()).reshape(256,256)
    ry, rx = np.gradient(residual,125)
    score = curvature*125+np.hypot(rx,ry)*.5
    score = gaussian_filter(score,2,mode='nearest')
    # 急な密度変化を避け、平野にも補間を安定させる面積枠を残す。
    normalized = score/max(float(np.mean(score)),1e-12)
    density = .35+.65*np.sqrt(np.clip(normalized,0,16))
    density /= density.mean()
    centers = sample_density(density,count-4,3581)
    samples = sample_density(density,max(131072,count*12),7211)
    for _ in range(12):
        labels = cKDTree(centers).query(samples)[1]
        mass = np.bincount(labels,minlength=len(centers))
        mean = np.column_stack([np.bincount(labels,weights=samples[:,k],minlength=len(centers)) for k in range(2)])
        occupied = mass>0
        centers[occupied] = .25*centers[occupied]+.75*mean[occupied]/mass[occupied,None]
    corners = np.array([[-16000,-16000],[-16000,16000],[16000,-16000],[16000,16000]])
    xy = canonical(np.vstack((centers,corners)))
    if len(np.unique(xy,axis=0)) != count: raise ValueError('量子化後に重複した制御点があります')
    separation = cKDTree(xy).query(xy,k=2)[0][:,1]
    return xy, dict(method='curvature-and-residual-weighted-cvt',count=count,
                    relaxationIterations=12,densityMin=float(density.min()),densityMax=float(density.max()),
                    minimumSeparationMeters=float(separation.min()),sigmaMeters=filtered.sigma_meters)
