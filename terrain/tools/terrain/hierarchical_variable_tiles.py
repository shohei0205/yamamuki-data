"""地域TINの親頂点共有、可変サイズ保存タイル、混在LOD接続の試験。"""
import json
import math
import sqlite3
import struct
import zlib
from collections import Counter, defaultdict
from pathlib import Path

import numpy as np
from scipy.spatial import Delaunay

from hierarchical_lod_compare import OUTPUT, REGIONS

# 全LODで同じ座標の刻みを使い、各タイルのuint16復元値を一致させる。
SIDE = 65520
LEVELS = [(0, 800), (2, 200), (4, 50), (5, 25)]


def cross(a, b, c):
    return (b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])


class Registry:
    def __init__(self, grid):
        self.points = []
        self.ids = {}
        self.grid = grid

    def add(self, p):
        key = (round(p[0]), round(p[1]))
        if key not in self.ids:
            self.ids[key] = len(self.points)
            # 切断で生じた頂点も生成用DEMから標高を取る。
            # 粗い面上の補間標高を固定すると、その誤差が詳細LODへ残る。
            x,y = key[0]*512/SIDE,key[1]*512/SIDE
            ix,iy = min(511,math.floor(x)),min(511,math.floor(y))
            u,v = x-ix,y-iy
            z = self.grid[iy,ix]*(1-u)*(1-v)+self.grid[iy,ix+1]*u*(1-v)+self.grid[iy+1,ix]*(1-u)*v+self.grid[iy+1,ix+1]*u*v
            self.points.append([key[0], key[1], round(float(z)*4)/4])
        return self.ids[key]


def clip(poly, axis, value, direction):
    result = []
    for a, b in zip(poly, poly[1:]+poly[:1]):
        ina, inb = direction*(a[axis]-value) >= 0, direction*(b[axis]-value) >= 0
        if ina:
            result.append(a)
        if ina != inb:
            t = (value-a[axis])/(b[axis]-a[axis])
            p = [a[i]+(b[i]-a[i])*t for i in range(3)]
            p[axis] = value
            result.append(p)
    return result


def partition(reg, faces, width):
    tiles = defaultdict(list)
    for face in faces:
        poly = [reg.points[i] for i in face]
        x0, x1 = min(p[0] for p in poly), max(p[0] for p in poly)
        y0, y1 = min(p[1] for p in poly), max(p[1] for p in poly)
        for y in range(y0//width, min((y1-1)//width, SIDE//width-1)+1):
            for x in range(x0//width, min((x1-1)//width, SIDE//width-1)+1):
                part = poly
                for axis, value, direction in [(0,x*width,1),(0,(x+1)*width,-1),(1,y*width,1),(1,(y+1)*width,-1)]:
                    part = clip(part, axis, value, direction)
                    if not part:
                        break
                ids = []
                for p in part:
                    i = reg.add(p)
                    if not ids or ids[-1] != i:
                        ids.append(i)
                if len(ids)>1 and ids[0] == ids[-1]:
                    ids.pop()
                for i in range(1,len(ids)-1):
                    f = (ids[0],ids[i],ids[i+1])
                    if cross(*(reg.points[j] for j in f))>0:
                        tiles[x,y].append(f)
    return dict(tiles)


def validate(points, tiles):
    edges = Counter()
    area = 0
    count = 0
    min_angle = 180
    for faces in tiles.values():
        for f in faces:
            p = [points[i] for i in f]
            a = cross(*p)
            assert a > 0, '反転・面積ゼロ'
            area += a
            count += 1
            for j in range(3):
                u, v = f[j], f[(j+1)%3]
                edges[u,v] += 1
                b, c = p[(j+1)%3], p[(j+2)%3]
                x, y = b[0]-p[j][0], b[1]-p[j][1]
                z, w = c[0]-p[j][0], c[1]-p[j][1]
                cosine = (x*z+y*w)/math.hypot(x,y)/math.hypot(z,w)
                min_angle = min(min_angle, math.degrees(math.acos(max(-1,min(1,cosine)))))
    bad = []
    for (a,b), n in edges.items():
        assert n == 1, '重複する有向辺'
        if edges[b,a] != 1:
            p,q = points[a],points[b]
            if not ((p[0]==q[0] and p[0] in (0,SIDE)) or (p[1]==q[1] and p[1] in (0,SIDE))):
                bad.append((a,b))
    assert math.isclose(area,2*SIDE*SIDE,rel_tol=0,abs_tol=.001), ('被覆面積',area)
    return {'triangles':count,'unmatchedInternalEdges':len(bad),'minAngle':min_angle,'doubledArea':area},bad


def error_metrics(grid, points, tiles):
    restored = np.full(grid.shape,np.nan)
    for faces in tiles.values():
        for f in faces:
            a,b,c = np.asarray([points[i] for i in f],dtype=float)
            a[:2] *= 512/SIDE;b[:2] *= 512/SIDE;c[:2] *= 512/SIDE
            x0,x1 = max(0,math.ceil(min(a[0],b[0],c[0]))),min(512,math.floor(max(a[0],b[0],c[0])))
            y0,y1 = max(0,math.ceil(min(a[1],b[1],c[1]))),min(512,math.floor(max(a[1],b[1],c[1])))
            if x0>x1 or y0>y1:
                continue
            x,y = np.meshgrid(np.arange(x0,x1+1),np.arange(y0,y1+1))
            area = cross(a,b,c)
            u = ((b[0]-x)*(c[1]-y)-(b[1]-y)*(c[0]-x))/area
            v = ((c[0]-x)*(a[1]-y)-(c[1]-y)*(a[0]-x))/area
            w = 1-u-v
            inside = (u>=-1e-8)&(v>=-1e-8)&(w>=-1e-8)
            restored[y[inside],x[inside]] = (u*a[2]+v*b[2]+w*c[2])[inside]
    assert np.isfinite(restored).all(), '評価格子に穴'
    d = restored-grid
    return {'maxError':float(np.abs(d).max()),'rmse':float(np.sqrt(np.mean(d*d)))}


def stitch(reg, leaves):
    """混在境界の節点を統一し、細側の高さを粗側の折れ線へ合わせる。"""
    counts = Counter((a,b) for _,_,fs in leaves for f in fs for a,b in zip(f,f[1:]+f[:1]))
    groups = defaultdict(list)
    for index,(width,key,faces) in enumerate(leaves):
        for fi,f in enumerate(faces):
            for a,b in zip(f,f[1:]+f[:1]):
                if counts[b,a]:
                    continue
                p,q = reg.points[a],reg.points[b]
                axis = 0 if p[0]==q[0] else 1 if p[1]==q[1] else -1
                if axis < 0:
                    raise AssertionError('内部に斜めの未接続辺')
                line = p[axis]
                if line in (0,SIDE):
                    continue
                groups[axis,line].append((index,fi,a,b,width))
    splits = defaultdict(dict)
    claims = {}
    for (axis,line), edges in groups.items():
        along = 1-axis
        nodes = {i for _,_,a,b,_ in edges for i in (a,b)}
        for index,fi,a,b,width in edges:
            p,q = reg.points[a],reg.points[b]
            lo,hi = sorted((p[along],q[along]))
            chain = sorted((i for i in nodes if lo<=reg.points[i][along]<=hi),key=lambda i:reg.points[i][along],reverse=p[along]>q[along])
            assert chain[0]==a and chain[-1]==b
            splits[index,fi][a,b] = chain
            for i in chain:
                t = (reg.points[i][along]-p[along])/(q[along]-p[along])
                z = round((p[2]+t*(q[2]-p[2]))*4)/4
                if i not in claims or claims[i][0]<width:
                    claims[i] = (width,z)
                elif claims[i][0]==width:
                    assert abs(claims[i][1]-z)<=.25, '同じ粗さの境界高が不一致'
    points = [p[:] for p in reg.points]
    for i,(_,z) in claims.items():
        points[i][2] = z
    result = {}
    added = 0
    for index,(width,key,faces) in enumerate(leaves):
        out = []
        for fi,f in enumerate(faces):
            chains = splits.get((index,fi),{})
            if not any(len(c)>2 for c in chains.values()):
                out.append(f)
                continue
            ring = []
            for a,b in zip(f,f[1:]+f[:1]):
                ring.extend(chains.get((a,b),[a,b])[:-1])
            # 同一直線上の境界点を飛ばさず、整数座標のまま耳切りする。
            while len(ring)>3:
                ears = []
                for j,b in enumerate(ring):
                    a,c = ring[j-1],ring[(j+1)%len(ring)]
                    pa,pb,pc = points[a],points[b],points[c]
                    area = cross(pa,pb,pc)
                    if area<=0:
                        continue
                    if any(cross(pa,pb,points[k])>=0 and cross(pb,pc,points[k])>=0 and cross(pc,pa,points[k])>=0 for k in ring if k not in (a,b,c)):
                        continue
                    ears.append((area,j,(a,b,c)))
                assert ears, '境界ポリゴンの再分割に失敗'
                _,j,face = max(ears)
                out.append(face);ring.pop(j)
            assert cross(*(points[i] for i in ring))>0
            out.append(tuple(ring))
        result[f'{width}:{key[0]}:{key[1]}'] = out
    return points,result,added


def generate():
    report = {'extent':16000,'regions':{},'scope':'16→8→4→2 km保存タイル。親頂点を保持。混在LODでは境界を粗い側の折れ線へ接続する試作。'}
    db = sqlite3.connect(OUTPUT/'variable-tiles.sqlite3')
    db.execute('CREATE TABLE IF NOT EXISTS meshes(region TEXT,level INTEGER,x INTEGER,y INTEGER,vertices BLOB,indices BLOB,PRIMARY KEY(region,level,x,y))')
    db.execute('CREATE TABLE IF NOT EXISTS shared_meshes(region TEXT,level INTEGER,x INTEGER,y INTEGER,refs BLOB,vertices BLOB,indices BLOB,PRIMARY KEY(region,level,x,y))')
    db.execute('CREATE TABLE IF NOT EXISTS mixed_meshes(region TEXT,width INTEGER,x INTEGER,y INTEGER,vertices BLOB,indices BLOB,PRIMARY KEY(region,width,x,y))')
    for region in REGIONS:
        original = json.loads((OUTPUT/(region+'.json')).read_text())
        grid = np.fromfile(OUTPUT/(region+'.f64'),dtype='<f8').reshape(513,513)
        reg = Registry(grid)
        all_levels = []
        prev = set()
        previous_decoded = {}
        for level,(source,target) in enumerate(LEVELS):
            for p in original[source]['Points']:
                reg.add([p[0]*SIDE/16000,p[1]*SIDE/16000,p[2]])
            ids = list(range(len(reg.points)))
            triangulation = Delaunay(np.asarray(reg.points)[:,:2])
            faces = [tuple(int(i) for i in f) if cross(*(reg.points[int(i)] for i in f))>0 else tuple(int(i) for i in reversed(f)) for f in triangulation.simplices]
            width = SIDE//(2**level)
            tiles = partition(reg,faces,width)
            stats,bad = validate(reg.points,tiles)
            assert not bad, ('同一LODの未接続辺',region,level,len(bad))
            used = {i for fs in tiles.values() for f in fs for i in f}
            assert prev <= used, '親頂点が詳細レベルから欠落'
            byte_count = 0
            shared_bytes = 0
            decoded_global = {}
            for (x,y),fs in tiles.items():
                local = sorted({i for f in fs for i in f})
                assert len(local)<65536
                remap = {v:i for i,v in enumerate(local)}
                vertices = b''.join(struct.pack('<3H',reg.points[i][0]-x*width,reg.points[i][1]-y*width,round(reg.points[i][2]*4)) for i in local)
                indices = b''.join(struct.pack('<3H',*(remap[i] for i in f)) for f in fs)
                vb,ib = zlib.compress(vertices,6),zlib.compress(indices,6)
                db.execute('INSERT OR REPLACE INTO meshes VALUES(?,?,?,?,?,?)',(region,level,x,y,vb,ib))
                assert zlib.decompress(vb)==vertices and zlib.decompress(ib)==indices
                for i,p in zip(local,struct.iter_unpack('<3H',zlib.decompress(vb))):
                    assert [p[0]+x*width,p[1]+y*width,p[2]/4]==reg.points[i], 'uint16のタイル別復元が不一致'
                byte_count += len(vb)+len(ib)
                # 共通IDによる親参照＋新規頂点。参照ID自体の容量も数える。
                refs = b''.join(struct.pack('<I',i) for i in local)
                fresh = b''.join(struct.pack('<3H',reg.points[i][0]-x*width,reg.points[i][1]-y*width,round(reg.points[i][2]*4)) for i in local if i not in prev)
                rb,fb = zlib.compress(refs,6),zlib.compress(fresh,6)
                shared_bytes += len(rb)+len(fb)+len(ib)
                db.execute('INSERT OR REPLACE INTO shared_meshes VALUES(?,?,?,?,?,?,?)',(region,level,x,y,rb,fb,ib))
                fresh_points = iter(struct.iter_unpack('<3H',zlib.decompress(fb)))
                for (i,) in struct.iter_unpack('<I',zlib.decompress(rb)):
                    if i in prev:
                        restored = previous_decoded[i]
                    else:
                        px,py,pz = next(fresh_points)
                        restored = [px+x*width,py+y*width,pz/4]
                    assert restored==reg.points[i]
                    if i in decoded_global:
                        assert decoded_global[i]==restored
                    decoded_global[i] = restored
                assert next(fresh_points,None) is None
            assert set(decoded_global)==used
            stats.update(error_metrics(grid,reg.points,tiles))
            view = {'widthKm':16/(2**level),'target':target,'tiles':[{'x':x,'y':y,'faces':fs} for (x,y),fs in tiles.items()],'points':[p[:] for p in reg.points],'side':SIDE}
            (OUTPUT/f'variable-{region}-{level}.json').write_text(json.dumps(view,separators=(',',':')),encoding='utf-8')
            stats.update({'tileCount':len(tiles),'uniqueVertices':len(used),'inheritedVertices':len(prev),'widthKm':view['widthKm'],'independentBytes':byte_count,'sharedBytes':shared_bytes})
            all_levels.append((width,tiles,stats));prev = used
            previous_decoded = decoded_global
            print(region,level,stats,flush=True)
        leaves = []
        for level in [1,2,3]:
            width,tiles,_ = all_levels[level]
            for key,fs in tiles.items():
                x,y = key[0]*width,key[1]*width
                if (level==1 and x<SIDE//2) or (level==2 and x>=SIDE//2 and y<SIDE//2) or (level==3 and x>=SIDE//2 and y>=SIDE//2):
                    leaves.append((width,key,fs))
        # 先に面積を確認し、T字接続を境界接続処理で解消する。
        before,bad = validate(reg.points,{str(i):v[2] for i,v in enumerate(leaves)})
        points,tiles,extra = stitch(reg,leaves)
        after,bad = validate(points,tiles)
        after.update(error_metrics(grid,points,tiles))
        assert not bad, ('混在LODの未接続辺',region,len(bad))
        # 混在側も、タイルごとのuint16保存・復元後のXYZを確認する。
        copies = defaultdict(set)
        for key,fs in tiles.items():
            width,x,y = map(int,key.split(':'))
            local = sorted({v for f in fs for v in f})
            remap = {v:i for i,v in enumerate(local)}
            vb = zlib.compress(b''.join(struct.pack('<3H',points[i][0]-x*width,points[i][1]-y*width,round(points[i][2]*4)) for i in local),6)
            ib = zlib.compress(b''.join(struct.pack('<3H',*(remap[i] for i in f)) for f in fs),6)
            db.execute('INSERT OR REPLACE INTO mixed_meshes VALUES(?,?,?,?,?,?)',(region,width,x,y,vb,ib))
            for i,p in zip(local,struct.iter_unpack('<3H',zlib.decompress(vb))):
                restored = [p[0]+x*width,p[1]+y*width,p[2]/4]
                assert restored==points[i]
                copies[i].add(struct.pack('<3d',*restored))
        assert all(len(v)==1 for v in copies.values())
        mixed = {'points':points,'tiles':[{'key':key,'faces':fs} for key,fs in tiles.items()],'side':SIDE}
        (OUTPUT/f'variable-{region}-mixed.json').write_text(json.dumps(mixed,separators=(',',':')),encoding='utf-8')
        report['regions'][region] = {'levels':[s for _,_,s in all_levels],'mixedBefore':before,'mixedAfter':after,'stitchVertices':extra,'maxSharedXYZGap':0}
    report['coordinateUnitMeters'] = 16000/SIDE
    report['heightUnitMeters'] = .25
    db.execute('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT)')
    db.execute('INSERT OR REPLACE INTO metadata VALUES(?,?)',('report',json.dumps(report,ensure_ascii=False)))
    db.commit();db.close()
    (OUTPUT/'variable-report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
    (OUTPUT/'variable.html').write_text(Path(__file__).with_suffix('.html').read_text(encoding='utf-8'),encoding='utf-8')


if __name__ == '__main__':
    generate()
