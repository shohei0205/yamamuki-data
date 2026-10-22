"""タイルから独立した頂点予算の評価と、元の面を保つタイル切り分け。"""
import numpy as np
from scipy.ndimage import gaussian_filter, maximum_filter, minimum_filter


def mixed_limits(relative):
    # 絶対誤差を主にし、小さい山の重みは最大1.25倍に抑える。
    inverse = .7 / 50. + .3 / np.clip(relative, 10., 100.)
    return 1. / np.minimum(inverse, 1.25 / 50.)


def broad_shape_limits(grid, valid, spacing):
    """2 kmの比高で形状誤差を正規化し、広い山腹も滑らかに重くする。"""
    grid=np.asarray(grid,dtype=np.float32);valid=np.asarray(valid,dtype=bool)
    # 欠損域のゼロ標高を、海岸付近の偽の比高として評価しない。
    weights=gaussian_filter(valid.astype(np.float32),max(1.,125./spacing))
    smooth=gaussian_filter(np.where(valid,grid,0.),max(1.,125./spacing))/np.maximum(weights,1e-6)
    filled=np.where(valid,smooth,np.nan)
    # 各方向の最大・最小を分離して、広域でも作業配列を抑える。
    size=2*max(1,round(1000./spacing))+1
    high=maximum_filter(np.where(valid,filled,-np.inf),size=size,mode='nearest')
    low=minimum_filter(np.where(valid,filled,np.inf),size=size,mode='nearest')
    relief=np.where(np.isfinite(high)&np.isfinite(low),high-low,0.)
    relief=gaussian_filter(relief,max(1.,500./spacing))
    presence=np.clip((relief-20.)/140.,0.,1.)
    presence=presence*presence*(3.-2.*presence)
    relative=np.clip(relief*.05,4.,50.)
    # 平野では50 mを維持し、山腹では比高の5%を主体にする。
    inverse=(1.-presence)/50.+presence*(.25/50.+.75/relative)
    limits=1./inverse
    limits[~valid]=50.;presence[~valid]=0.
    return limits.astype(np.float32),presence.astype(np.float32)


def landform_limits(grid, valid, spacing, relative):
    """広い周辺平均から盛り上がる地形を評価し、従来の誤差を最大3.5倍重くする。"""
    valid=np.asarray(valid,dtype=bool);grid=np.asarray(grid,dtype=np.float32)
    def smooth(scale):
        sigma=max(1.,scale/spacing)
        weights=gaussian_filter(valid.astype(np.float32),sigma)
        values=gaussian_filter(np.where(valid,grid,0.),sigma)
        return values/np.maximum(weights,1e-6)
    terrain=smooth(250.);background=smooth(6000.)
    size=2*max(1,round(8000./spacing))+1
    high=maximum_filter(np.where(valid,terrain,-np.inf),size=size,mode='nearest')
    low=minimum_filter(np.where(valid,terrain,np.inf),size=size,mode='nearest')
    relief=np.where(np.isfinite(high)&np.isfinite(low),high-low,0.)
    positive=np.maximum(terrain-background,0.)
    normalized=positive/np.maximum(relief,100.)
    weight=np.clip((normalized-.02)/.20,0.,1.)
    weight=weight*weight*(3.-2.*weight)
    # 低い起伏へ頂点を回さず、重みの境界を約1 kmでぼかす。
    presence=np.clip((positive-20.)/80.,0.,1.);presence=presence*presence*(3.-2.*presence)
    weight=gaussian_filter(weight*presence,max(1.,1000./spacing));weight[~valid]=0.
    base=mixed_limits(relative)
    return (base/(1.+2.5*weight)).astype(np.float32),weight.astype(np.float32)


def movable_priority(error, limits, valid):
    """固定外周と欠損域を除き、追加点で減らせる最大の正規化誤差を返す。"""
    movable=np.asarray(valid,dtype=bool).copy()
    movable[[0,-1],:]=False;movable[:,[0,-1]]=False
    return float(np.max(np.abs(error[movable])/limits[movable])) if movable.any() else 0.


def allocation_score(error, relative, valid, terrain_weight=None, shape_limits=None, feature_balance=False):
    """同面積の標高格子で残差を積分。追加による実際の減少量ではない。"""
    e = np.maximum(np.abs(error) - 2., 0.)
    values = .7 * e / 50. + .3 * np.minimum(e / np.clip(relative, 10., 100.), 4.)
    if shape_limits is not None:
        values = np.minimum(e / np.maximum(shape_limits, 1.), 8.)
    base = float(np.where(valid, values, 0.).mean())
    if terrain_weight is None or not valid.any():
        return base
    weights = np.where(valid, np.clip(terrain_weight, 0., 1.), 0.)
    # 山地が少ない地域も評価するが、狭い一点へ予算を集中させない。
    numerator=.5 * float((values * weights).sum())
    previous=base+numerator/max(float(weights.sum()),.1*int(valid.sum()))
    if not feature_balance:return previous
    feature=numerator/max(float(weights.sum()),.005*int(valid.sum()))
    return base+min(feature,2.*previous)



def triangulate_clipped_polygon(ids, points):
    """丸めでわずかに凹んだ多角形も、面を裏返さずに三角形へ分ける。"""
    ids = list(ids)
    if len(ids) < 3: return []
    xy = {i: np.asarray(points[i])[:2] for i in ids}
    def cross(a, b, c):
        u, v = xy[b] - xy[a], xy[c] - xy[a]
        return float(u[0] * v[1] - u[1] * v[0])
    signed = sum(xy[a][0]*xy[b][1]-xy[a][1]*xy[b][0] for a,b in zip(ids,ids[1:]+ids[:1]))
    if signed < 0: ids.reverse()
    output = []
    while len(ids) > 3:
        for i, b in enumerate(ids):
            a, c = ids[i-1], ids[(i+1)%len(ids)]
            if cross(a,b,c) <= 0: continue
            if any(cross(a,b,d)>=0 and cross(b,c,d)>=0 and cross(c,a,d)>=0 for d in ids if d not in (a,b,c)): continue
            output.append([a,b,c]);ids.pop(i);break
        else:
            # 面積ゼロの辺上点は取り除ける。負の面を反転すると重なりになる。
            for i,b in enumerate(ids):
                if cross(ids[i-1],b,ids[(i+1)%len(ids)]) == 0:
                    ids.pop(i);break
            else: raise ValueError('クリップした多角形を三角形分割できません')
    if len(ids)==3 and cross(*ids)>0: output.append(ids)
    return output


def _split_crossing_mesh(points, faces, width=32000., tile_width=8000.):
    """三角形を長方形でクリップし、共有交点の高さを一度だけ確定する。"""
    points = np.asarray(points, dtype=float)
    result = {}
    heights = {}
    divisions = round(width / tile_width)
    for face in faces:
        tri = points[face]
        lo = np.maximum(0, np.floor(tri[:, :2].min(axis=0) / tile_width).astype(int))
        hi = np.minimum(divisions - 1, np.floor(tri[:, :2].max(axis=0) / tile_width).astype(int))
        basis = np.column_stack([tri[1, :2] - tri[0, :2], tri[2, :2] - tri[0, :2]])
        inverse = np.linalg.inv(basis)
        for tx in range(lo[0], hi[0] + 1):
            for ty in range(lo[1], hi[1] + 1):
                polygon = [v.copy() for v in tri[:, :2]]
                for axis, bound, sign in [(0, tx * tile_width, 1), (0, (tx + 1) * tile_width, -1), (1, ty * tile_width, 1), (1, (ty + 1) * tile_width, -1)]:
                    output = []
                    for a, b in zip(polygon, polygon[1:] + polygon[:1]):
                        inside_a, inside_b = sign * (a[axis] - bound) >= 0, sign * (b[axis] - bound) >= 0
                        if inside_a:
                            output.append(a)
                        if inside_a != inside_b:
                            t = (bound - a[axis]) / (b[axis] - a[axis])
                            q = a + t * (b - a)
                            q[axis] = bound
                            output.append(q)
                    polygon = output
                    if not polygon:
                        break
                ids = []
                tile = result.setdefault((tx, ty), dict(points=[], faces=[], lookup={}))
                for xy in polygon:
                    # 2進数で表現できる座標に揃え、Float32格納後も共有辺を一致させる。
                    key = tuple(np.rint(xy * 256).astype(int))
                    if key not in heights:
                        uv = inverse @ (np.asarray(key) / 256. - tri[0, :2])
                        z = tri[0, 2] + uv @ (tri[1:, 2] - tri[0, 2])
                        heights[key] = round(float(z) * 1024) / 1024
                    if key not in tile['lookup']:
                        tile['lookup'][key] = len(tile['points'])
                        tile['points'].append([key[0] / 256 - tx * tile_width, key[1] / 256 - ty * tile_width, heights[key]])
                    index = tile['lookup'][key]
                    if not ids or ids[-1] != index:
                        ids.append(index)
                if len(ids) > 1 and ids[0] == ids[-1]:
                    ids.pop()
                tile['faces'].extend(triangulate_clipped_polygon(ids, tile['points']))
    return {k: dict(points=np.asarray(t['points']), faces=np.asarray(t['faces'], dtype=np.uint32)) for k, t in result.items() if t['faces']}



def split_mesh(points,faces,width=32000.,tile_width=8000.):
    """保存境界をまたがない面をまとめ、交差面だけ従来のクリップで処理する。"""
    points=np.asarray(points,float);faces=np.asarray(faces,np.uint32)
    if np.any(np.abs(points[:,:2]*256-np.rint(points[:,:2]*256))>1e-7) or np.any(np.abs(points[:,2]*1024-np.rint(points[:,2]*1024))>1e-7):return _split_crossing_mesh(points,faces,width,tile_width)
    divisions=round(width/tile_width);q=points[faces,:2];lo=np.maximum(0,np.floor(q.min(axis=1)/tile_width).astype(int));hi=np.minimum(divisions-1,np.floor(q.max(axis=1)/tile_width).astype(int));inside=np.all(lo==hi,axis=1)
    crossing=_split_crossing_mesh(points,faces[~inside],width,tile_width);result={};owners=lo[:,1]*divisions+lo[:,0]
    for owner in np.unique(owners[inside]):
        x=int(owner%divisions);y=int(owner//divisions);selected=faces[inside&(owners==owner)];used,indices=np.unique(selected,return_inverse=True);p=points[used].copy();p[:,:2]-=[x*tile_width,y*tile_width];f=indices.reshape(-1,3)
        part=crossing.pop((x,y),None)
        if part is not None:
            lookup={tuple(v[:2]):i for i,v in enumerate(p)};extra=[];remap=[]
            for v in part['points']:
                key=tuple(v[:2]);index=lookup.get(key)
                if index is None:index=len(p)+len(extra);lookup[key]=index;extra.append(v)
                remap.append(index)
            if extra:p=np.vstack([p,extra])
            f=np.vstack([f,np.asarray(remap)[part['faces']]])
        result[x,y]=dict(points=p,faces=np.asarray(f,np.uint32))
    result.update(crossing);return result
