"""3地域の頂点予算を残差の面積積分で共有し、生成後に8 kmタイルへ分割する。"""
import argparse, json, sqlite3, struct, subprocess, tempfile, zlib, time
from contextlib import ExitStack, closing
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.ndimage import map_coordinates, gaussian_filter
import compare_priority_from_start as compare
from weighted_mesh_allocation import mixed_limits, allocation_score, split_mesh, broad_shape_limits, landform_limits, movable_priority

ROOT = compare.ROOT


def residual(points, faces, grid):
    # 接続が一致する場合だけSciPyの点位置検索を使い、巨大な細長い面の全外接矩形を走査しない。
    from scipy.spatial import Delaunay
    tri=Delaunay(np.asarray(points)[:,:2])
    canonical=lambda f:{tuple(sorted(map(int,v))) for v in f}
    matched=canonical(tri.simplices)
    different=[ids for ids in faces if tuple(sorted(map(int,ids))) not in matched]
    n=len(grid);out=np.empty_like(grid,dtype=float)
    for y in range(0,n,64):
        yy,xx=np.mgrid[y:min(n,y+64),:n];xy=np.column_stack([xx.ravel(),yy.ravel()]);ids=tri.find_simplex(xy)
        if (ids<0).any():raise ValueError('未被覆の格子点')
        uv=np.einsum('nij,nj->ni',tri.transform[ids,:2],xy-tri.transform[ids,2]);w=np.c_[uv,1-uv.sum(axis=1)]
        out[y:y+64]=(points[tri.simplices[ids],2]*w).sum(axis=1).reshape(yy.shape)-grid[y:y+64]
    # 同一円上の点などで対角線が違う領域だけ、実際の接続で上書きする。
    for ids in different:
        a,b,c=points[ids];u=b[:2]-a[:2];v=c[:2]-a[:2];den=u[0]*v[1]-u[1]*v[0]
        if den<=1e-7:raise ValueError('反転または退化面')
        lo=np.maximum(0,np.ceil(points[ids,:2].min(axis=0)-1e-7).astype(int));hi=np.minimum(n-1,np.floor(points[ids,:2].max(axis=0)+1e-7).astype(int))
        for y in range(lo[1],hi[1]+1,64):
            yy,xx=np.mgrid[y:min(hi[1]+1,y+64),lo[0]:hi[0]+1];dx=xx-a[0];dy=yy-a[1]
            wb=(dx*v[1]-dy*v[0])/den;wc=(u[0]*dy-u[1]*dx)/den;inside=(wb>=-1e-8)&(wc>=-1e-8)&(wb+wc<=1+1e-8)
            predicted=a[2]+wb*(b[2]-a[2])+wc*(c[2]-a[2]);out[yy[inside],xx[inside]]=predicted[inside]-grid[yy[inside],xx[inside]]
    return out


def boundary_seeds(grid, tolerance):
    n=len(grid);result=set()
    for axis,value in [(0,0),(0,n-1),(1,0),(1,n-1)]:
        line=grid[:,value] if axis==0 else grid[value,:];keep={0,n-1};pending=[(0,n-1)]
        while pending:
            a,b=pending.pop()
            if b-a<2:continue
            error=np.abs(line[a+1:b]-np.linspace(line[a],line[b],b-a+1)[1:-1]);i=a+1+int(error.argmax())
            if error.max()>tolerance:keep.add(i);pending.extend([(a,i),(i,b)])
        for i in keep:result.add((value,i) if axis==0 else (i,value))
    return sorted(result)


def close_client(region):
    process=region.get('process')
    if process is not None:
        process.stdin.close()
        try:process.wait(timeout=20)
        except subprocess.TimeoutExpired:process.terminate();process.wait(timeout=10)
        process.stdout.close();region['stderr'].close()


def prepare(area, source, folder, width=32000., shape_aware=False, landform_aware=False):
    key = next(v for v in area['levels'] if v['level'] == 5)['keys'][0]
    cx, cy = compare.national.project(area['lon'], area['lat'])
    if width != 32000.: key = [7, int(np.floor(cx/width)), int(np.floor(cy/width))]
    _, x, y = key; n = round(width/31.25)+1; spacing = width / (n - 1)
    offset = [float(cx - (x + .5) * width), float(cy - (y + .5) * width)]
    original = compare.national.source_samples
    def shifted_samples(xx, yy):
        if width == 32000.:return original(xx + offset[0], yy + offset[1])
        values=np.empty(xx.shape,dtype=np.float32);valid=np.empty(xx.shape,dtype=bool)
        for start in range(0,len(xx),64):
            values[start:start+64],valid[start:start+64]=original(xx[start:start+64]+offset[0],yy[start:start+64]+offset[1])
        return values,valid
    compare.national.source_samples = shifted_samples
    try:
        grid, limits, gx, gy = compare.quality.source_fields(5, x, y, n, width_meters=width, error_meters=100.)
        axis = np.arange(n) * spacing
        _, valid = compare.national.source_samples(*np.meshgrid(x * width + axis, y * width + axis))
    finally:
        compare.national.source_samples = original
    if width == 32000. and not landform_aware:
        raw = zlib.decompress(source.execute('SELECT geometry FROM tiles WHERE level=? AND x=? AND y=?', key).fetchone()[0])
        nv = struct.unpack_from('<I', raw, 8)[0]
        old = np.frombuffer(raw, '<f4', nv * 3, 16).reshape(-1, 3)
        boundary = old[np.any((old[:, :2] == 0) | (old[:, :2] == width), axis=1), :2]
        seed = sorted({tuple(map(int, v)) for v in np.rint(boundary / spacing).astype(int)})
    else:
        seed = boundary_seeds(grid, 12.5)
    folder.mkdir()
    height_offset = min(0., float(grid.min()))
    Image.fromarray(np.rint((grid - height_offset) * 4).astype('uint16')).save(folder / 'height.png')
    for mode, field in [('relative', limits), ('balanced', mixed_limits(limits))]:
        Image.fromarray(np.maximum(1, np.rint(field * 100)).astype('uint16')).save(folder / (mode + '.png'))
    shape_limits=None
    if shape_aware or landform_aware:
        if landform_aware:
            # 表示範囲の外も参照し、範囲端を山と誤認しない。
            field_spacing=250.;pad=112;core=round(width/field_spacing)+1
            field_axis=(np.arange(core+2*pad)-pad)*field_spacing
            xx,yy=np.meshgrid(x*width+field_axis+offset[0],y*width+field_axis+offset[1])
            halo,halo_valid=original(xx,yy)
            _,field_weight=landform_limits(halo,halo_valid,field_spacing,np.full_like(halo,50.))
            axis=np.arange(n)*spacing/field_spacing+pad;fy,fx=np.meshgrid(axis,axis,indexing='ij')
            shape_weight=map_coordinates(field_weight,[fy,fx],order=1).astype(np.float32);shape_weight[~valid]=0.
            shape_limits=(mixed_limits(limits)/(1.+2.5*shape_weight)).astype(np.float32)
            shape_limits[~valid]=655.35
        else:
            shape_limits,shape_weight=broad_shape_limits(grid,valid,spacing)
        Image.fromarray(np.maximum(1,np.rint(shape_limits*100)).astype('uint16')).save(folder/'balanced.png')
    smooth = gaussian_filter(grid, 2.); dy, dx = np.gradient(smooth, spacing); terrain_weight = np.clip((np.hypot(dx, dy) - .035) / .115, 0., 1.)
    return dict(shape_limits=shape_limits, shape_aware=shape_aware, landform_aware=landform_aware, width=width, terrain_weight=(shape_weight if (shape_aware or landform_aware) else terrain_weight).astype(np.float32), area=area, key=key, offset=offset, n=n, spacing=spacing, grid=grid.astype(np.float32), limits=limits.astype(np.float32), gx=gx.astype(np.float32), gy=gy.astype(np.float32), valid=valid, seed=seed, folder=folder, height_offset=height_offset)


def grow(region, mode, target, previous=None, force=False):
    folder = region['folder']
    seeds = region['seed'] if previous is None else previous['p'][:, :2].astype(int).tolist()
    (folder / 'seed.json').write_text(json.dumps(seeds), encoding='utf-8')
    command = [compare.national.BINARY, '--input', str(folder / 'height.png'), '--output', str(folder / 'mesh.json'), '--scale', '.25', '--offset', str(region['height_offset']), '--error-limits', str(folder / (mode + '.png')), '--max-vertices', str(target), '--min-vertices', str(target if force else 4), '--seed-points', str(folder / 'seed.json'), '--lock-seed-boundary']
    if region.get('incremental'):
        if 'process' not in region:
            command[command.index('--max-vertices')+1]=str(region['maximum_budget'])
            region['stderr']=open(folder/'incremental.err','w',encoding='utf-8')
            region['process']=subprocess.Popen(command+['--incremental']+(['--fill-budget'] if region.get('fill_budget') else []),stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=region['stderr'],text=True,encoding='utf-8')
            region['stack'].callback(close_client,region)
        process=region['process'];process.stdin.write(str(target)+'\n');process.stdin.flush()
        reply=process.stdout.readline()
        if not reply:raise RuntimeError((folder/'incremental.err').read_text(encoding='utf-8')[-2000:])
        json.loads(reply)
    else:
        subprocess.run(command, check=True, capture_output=True, text=True)

    mesh = json.loads((folder / 'mesh.json').read_text(encoding='utf-8'))
    p = np.asarray(mesh['vertices'], dtype=float); f = np.asarray(mesh['triangles'], dtype=np.uint32)
    tri = p[f, :2]; cross = np.linalg.det(np.stack([tri[:, 1] - tri[:, 0], tri[:, 2] - tri[:, 0]], axis=2)); f[cross < 0] = f[cross < 0][:, [0, 2, 1]]
    error = residual(p, f, region['grid']); v = np.abs(error[region['valid']])
    mean_score=allocation_score(error, region['limits'], region['valid'], region['terrain_weight'] if region.get('terrain_aware') else None, shape_limits=region.get('shape_limits'))
    if region.get('landform_aware'):
        score=movable_priority(error,region['shape_limits'],region['valid'])
    else:
        score=mean_score
    return dict(p=p, f=f, score=score, meanScore=mean_score, maximum=float(v.max()), rmse=float(np.sqrt((v * v).mean())), weightedMaximum=float((np.abs(error) / region['limits'])[region['valid']].max()), reached=bool(mesh['toleranceReached']))


def encode_mask(valid):
    # 標高格子は南から北、描画用マスクは北から南の行順で保存する。
    return zlib.compress(np.packbits(valid[::-1], bitorder='little').tobytes(), 9)


def encode_surface_mask(valid):
    # 欠損境界は海岸線ではない。描画用には全面を有効とする。
    return encode_mask(np.ones_like(valid,dtype=bool))

def save(mode, regions, meshes, manifest, budget, trace, variant=None, rebuild_topology=True):
    width=regions[0]['width'];suffix='128' if width==128000 else '';mode=variant or (('landform128' if width==128000 else 'landform') if regions[0].get('landform_aware') else 'shape128' if regions[0].get('shape_aware') else mode+suffix)
    path = ROOT / f'build/terrain-shared-allocation-{mode}.sqlite3'
    with closing(sqlite3.connect(path)) as db, db:
        db.executescript('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE IF NOT EXISTS tiles(level,x,y,geometry,hints,mask,stats,PRIMARY KEY(level,x,y));')
        db.execute('DELETE FROM tiles')
        m = json.loads(json.dumps(manifest)); m['levels'] = [dict(level=3, widthMeters=8000, maxErrorMeters=50)]
        m.update(coastlineInMesh=False,missingElevationPlaceholderMeters=0,priorityComparison=True, priorityMode='unified', mountainCentered=True, sharedAllocation=dict(mode=mode, totalBudget=budget, sourceVertices=sum(len(v['p']) for v in meshes), trace=trace, widthMeters=width, evaluationSpacing=31.25, allocationDomain='three-disconnected-equal-area-regions', maskRowOrder='north-to-south'))
        if regions[0].get('shape_aware'):m['sharedAllocation']['shapePriority']=dict(reliefDiameterMeters=2000,reliefFraction=.05,relativeMinimumMeters=4,relativeMaximumMeters=50,absoluteWeight=.25,relativeWeight=.75,noiseIgnoreMeters=2,presenceReliefRangeMeters=[20,160])
        if regions[0].get('landform_aware'):m['sharedAllocation']['landformPriority']=dict(backgroundSigmaMeters=6000,reliefDiameterMeters=16000,maximumGain=3.5,weightSigmaMeters=1000,fieldSpacingMeters=250,haloMeters=28000,boundaryToleranceMeters=12.5,regionSelection='maximum-movable-land-normalized-error',allocationBatchVertices=regions[0].get('allocation_step',8192))
        for region, mesh, area in zip(regions, meshes, m['reliefRegions']):
            area['comparisonOffset'] = region['offset']; area['levels'] = []
            world = mesh['p'].copy(); world[:, :2] *= region['spacing']
            pieces = split_mesh(world, mesh['f'],width=width); keys = []; geometry_bytes = 0; stored = 0
            source_rank = {tuple(np.rint(v[:2] * 256).astype(int)): i / max(1, len(world) - 1) for i, v in enumerate(world)}
            for (tx, ty), part in pieces.items():
                used = np.unique(part['faces']); remap = np.zeros(len(part['points']), dtype=np.uint32); remap[used] = np.arange(len(used)); p = part['points'][used]; f = remap[part['faces']]
                xy = p[:, :2] + [tx * 8000, ty * 8000]; ranks = np.array([source_rank.get(tuple(np.rint(v * 256).astype(int)), -1.) for v in xy], dtype='<f4')
                ranks[np.any((p[:, :2] == 0) | (p[:, :2] == 8000), axis=1)] = -1
                nv = len(p); assert nv < 65536
                g = zlib.compress(struct.pack('<4sHHII', b'YNT3', 257, 257, nv, f.size) + p.astype('<f4').tobytes() + f.astype('<u2').tobytes() + ranks.tobytes(), 9)
                uv = xy / region['spacing']; slopes = np.column_stack([map_coordinates(region['gx'], [uv[:, 1], uv[:, 0]], order=1), -map_coordinates(region['gy'], [uv[:, 1], uv[:, 0]], order=1)])
                h = zlib.compress(struct.pack('<4sIf', b'YNH1', nv, 8.) + np.rint(np.clip(slopes, -8, 8) / 8 * 32767).astype('<i2').tobytes(), 9)
                valid = region['valid'][ty*256:(ty+1)*256+1, tx*256:(tx+1)*256+1]
                mask = encode_surface_mask(valid)
                key = [3, region['key'][1] * round(width/8000) + tx, region['key'][2] * round(width/8000) + ty]; keys.append(key)
                db.execute('INSERT INTO tiles VALUES(?,?,?,?,?,?,?)', (*key, g, h, mask, json.dumps(dict(vertices=nv))))
                geometry_bytes += len(g); stored += nv
            area['levels'].append(dict(level=3, widthMeters=8000, tileCount=len(keys), keys=keys, addedPositions=[], priorityMetrics=dict(vertices=stored, sourceVertices=len(world), maximum=mesh['maximum'], rmse=mesh['rmse'], weightedMaximum=mesh['weightedMaximum'], geometryBytes=geometry_bytes, allocationScore=mesh['score'])))
        db.execute('INSERT OR REPLACE INTO metadata VALUES("manifest",?)', (json.dumps(m),))
    if rebuild_topology and (width==128000 or regions[0].get('landform_aware')):
        from uncut_mesh_topology import rebuild
        rebuild(path)
    print('保存', mode, [(r['area']['id'], len(v['p']), round(v['rmse'], 2)) for r, v in zip(regions, meshes)], flush=True)


def main(budget, step, terrain_aware=False, width=32000., incremental=False, binary=None, budgets=None, shape_aware=False, landform_aware=False):
    compare.initialize()
    if binary:compare.national.BINARY=str(Path(binary).resolve())
    with sqlite3.connect(ROOT / 'build/terrain-regional-priority-levels-v5.sqlite3') as source, tempfile.TemporaryDirectory(dir=ROOT/'build', prefix='shared-allocation-') as folder, ExitStack() as stack:
        manifest = json.loads(source.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
        regions=[]
        for area in manifest['reliefRegions']:
            print('標高を準備',area['id'],width,flush=True);regions.append(prepare(area,source,Path(folder)/area['id'],width,shape_aware,landform_aware))
        for r in regions:r.update(incremental=incremental,maximum_budget=budget,stack=stack,fill_budget=bool(budgets) or shape_aware or landform_aware,allocation_step=step)
        if not terrain_aware:
            baseline = [grow(r, 'relative', budget // len(regions), force=True) for r in regions]
            save('relative', regions, baseline, manifest, budget, [])
        for r in regions: r['terrain_aware'] = terrain_aware
        meshes = [grow(r, 'balanced', max(round(256*(width/32000)**2), len(r['seed']) + 4)) for r in regions]
        trace = []; stopped = set(); checkpoints = sorted(set(budgets or [])); saved = set()
        while sum(len(v['p']) for v in meshes) < budget:
            eligible = [i for i, v in enumerate(meshes) if i not in stopped and (checkpoints or shape_aware or landform_aware or (not v['reached'] and v['score'] > .03))]
            if not eligible: break
            i = max(eligible, key=lambda k: (meshes[k]['score'], -k)); old = len(meshes[i]['p'])
            total = sum(len(v['p']) for v in meshes)
            next_checkpoint = next((v for v in checkpoints if v > total), budget)
            target = old + min(step, budget - total, next_checkpoint - total)
            meshes[i] = grow(regions[i], 'balanced', target, meshes[i])
            if len(meshes[i]['p']) == old: stopped.add(i)
            trace.append(dict(region=regions[i]['area']['id'], vertices=len(meshes[i]['p']), score=meshes[i]['score']))
            total = sum(len(v['p']) for v in meshes)
            if total in checkpoints:
                save('terrain', regions, meshes, manifest, total, trace, variant=f'terrain128-b{total}')
                saved.add(total)
            print('配分', trace[-1], '総頂点', sum(len(v['p']) for v in meshes), flush=True)
            (ROOT/('build/terrain-landform-allocation.progress.json' if landform_aware else 'build/terrain-shape-allocation.progress.json' if shape_aware else f'build/terrain-shared-allocation-{round(width/1000)}.progress.json')).write_text(json.dumps(dict(widthMeters=width,budget=budget,vertices=sum(len(v['p']) for v in meshes),regions={r['area']['id']:len(v['p']) for r,v in zip(regions,meshes)})),encoding='utf-8')
        if checkpoints:
            for checkpoint in checkpoints:
                if checkpoint not in saved:
                    save('terrain', regions, meshes, manifest, checkpoint, trace, variant=f'terrain128-b{checkpoint}')
        else:
            save('terrain' if terrain_aware else 'balanced', regions, meshes, manifest, budget, trace)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(); parser.add_argument('--budgets', type=int, nargs='+'); parser.add_argument('--budget', type=int); parser.add_argument('--step', type=int); parser.add_argument('--terrain-aware', action='store_true');parser.add_argument('--shape-aware',action='store_true');parser.add_argument('--landform-aware',action='store_true'); parser.add_argument('--width-km',type=int,choices=[32,128],default=32);parser.add_argument('--incremental',action='store_true');parser.add_argument('--binary');args = parser.parse_args()
    if args.landform_aware:
        if args.shape_aware or args.budgets:parser.error('地形の盛り上がり評価は単一予算で指定してください')
        args.terrain_aware=True
    if args.shape_aware:
        if args.width_km!=128 or args.budgets:parser.error('形状比高比較は128 km・単一予算で実行してください')
        args.terrain_aware=True
    if args.budgets:
        if args.width_km != 128 or not args.terrain_aware or min(args.budgets)<48000:parser.error('複数予算は128 km・山地評価・48000以上を指定してください')
        args.budget=max(args.budgets)
    if args.budget is None:args.budget=round(12000*(args.width_km/32)**2)
    if args.step is None:args.step=round(512*(args.width_km/32)**2)
    if args.incremental and args.binary is None:args.binary=str(ROOT/'build/terrain-rust-incremental-target/release/terrain-adaptive-tin.exe')
    if args.budget < max(1500,round(3*256*(args.width_km/32)**2)) or args.budget > 1000000 or args.step < 1: parser.error('予算は1500～1000000、追加単位は1以上を指定してください')
    main(args.budget, args.step, args.terrain_aware, args.width_km*1000., args.incremental, args.binary, args.budgets, args.shape_aware, args.landform_aware)
