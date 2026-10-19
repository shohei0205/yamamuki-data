"""誤差が大きい地点へ頂点を追加し、不規則な三角形で地形を近似する試作。"""
import argparse
import hashlib
import json
import lzma
from pathlib import Path
import sqlite3
import time

import numpy as np
from polygon_dem import read_grid,encode_mesh,decode_mesh
from ridges import tile_position
from polygon_dem import HEADER


def encode_irregular(vertices,faces,step=5):
    from scipy.spatial import Delaunay
    generated=Delaunay(vertices[:,:2]).simplices.astype('<u4') if len(vertices)>=3 else np.empty((0,3),'<u4')
    selected={tuple(sorted(face)) for face in faces.tolist()}
    keep=np.asarray([tuple(sorted(face)) in selected for face in generated.tolist()],dtype=np.uint8)
    points=vertices.copy();points[:,2]=np.rint(points[:,2]/step)
    delta=np.diff(points.astype(np.int32),axis=0,prepend=np.zeros((1,3),np.int32))
    if delta.size and (delta.min()<-32768 or delta.max()>32767):raise ValueError('頂点差分が保存範囲を超えています')
    has_mask=int(not keep.all())
    mask=np.packbits(keep,bitorder='little').tobytes() if has_mask else b''
    digest=hashlib.sha256(generated.tobytes()).digest()
    return HEADER.pack(b'YMP3',257,step,len(vertices),len(generated),has_mask)+lzma.compress(delta.astype('<i2').tobytes()+mask+digest)


def decode_irregular(body):
    from scipy.spatial import Delaunay
    magic,size,step,nv,nt,has_mask=HEADER.unpack_from(body)
    if magic!=b'YMP3' or size!=257 or nv>66049 or nt>150000 or has_mask not in (0,1) or step<=0:raise ValueError('不規則メッシュの形式が不正です')
    raw=lzma.decompress(body[HEADER.size:]);mask_size=(nt+7)//8 if has_mask else 0
    if len(raw)!=nv*6+mask_size+32:raise ValueError('不規則メッシュの長さが不正です')
    vertices=np.frombuffer(raw[:nv*6],dtype='<i2').reshape(-1,3).cumsum(axis=0).astype(np.float32)
    if vertices.size and (vertices[:,:2].min()<0 or vertices[:,:2].max()>256):raise ValueError('頂点座標が範囲外です')
    faces=Delaunay(vertices[:,:2]).simplices.astype('<u4') if nv>=3 else np.empty((0,3),'<u4')
    if len(faces)!=nt or hashlib.sha256(faces.tobytes()).digest()!=raw[-32:]:raise ValueError('三角形の復元が一致しません。生成時と同じ三角形分割が必要です')
    if has_mask:
        keep=np.unpackbits(np.frombuffer(raw[nv*6:nv*6+mask_size],dtype=np.uint8),bitorder='little')[:nt].astype(bool)
        faces=faces[keep]
    vertices[:,2]*=step
    return vertices,faces


def simplify_irregular(grid,tolerance,*,max_iterations=200,batch_size=512):
    from scipy.spatial import Delaunay
    import shapely
    from shapely.geometry import box
    if grid.ndim!=2 or grid.shape[0]!=grid.shape[1] or grid.shape[0]<2 or not np.isfinite(tolerance) or tolerance<0:raise ValueError('正方形の標高配列と 0 以上の有限な誤差上限が必要です')
    n=grid.shape[0]-1
    valid=np.isfinite(grid)
    if valid.sum()<3:return np.empty((0,3),np.float32),np.empty((0,3),np.uint32),0.
    y,x=np.indices(grid.shape);points=np.column_stack([x.ravel(),y.ravel()])
    neighbor=valid.copy()
    neighbor[1:]&=valid[:-1];neighbor[:-1]&=valid[1:];neighbor[:,1:]&=valid[:,:-1];neighbor[:,:-1]&=valid[:,1:]
    edge=~neighbor&(x>0)&(x<n)&(y>0)&(y<n)
    selected=set(np.flatnonzero(valid&edge))
    def profile(values):
        indices=set();finite=np.isfinite(values)
        runs=np.flatnonzero(np.diff(np.pad(finite.astype(np.int8),(1,1))))
        def visit(a,b):
            indices.add(a);indices.add(b)
            if b-a<2:return
            estimate=np.linspace(values[a],values[b],b-a+1)
            error=np.abs(values[a:b+1]-estimate);offset=int(np.argmax(error))
            if error[offset]>tolerance*.5:
                visit(a,a+offset);visit(a+offset,b)
        for a,b in zip(runs[::2],runs[1::2]):visit(int(a),int(b-1))
        return indices
    # 境界も固定間隔にせず、同じ標高列から両側のタイルが同じ点を選ぶ。
    for i in profile(grid[0]):selected.add(i)
    for i in profile(grid[-1]):selected.add(n*(n+1)+i)
    for i in profile(grid[:,0]):selected.add(i*(n+1))
    for i in profile(grid[:,-1]):selected.add(i*(n+1)+n)
    finite=np.flatnonzero(valid)
    interior=np.flatnonzero(valid&(x>0)&(x<n)&(y>0)&(y<n))
    if len(interior):
        selected.add(int(interior[np.argmax(grid.ravel()[interior])]))
        selected.add(int(interior[np.argmin(grid.ravel()[interior])]))
    if len(selected)<3:return np.empty((0,3),np.float32),np.empty((0,3),np.uint32),0.
    for iteration in range(max_iterations):
        ids=np.asarray(sorted(selected));xy=points[ids]
        try:tri=Delaunay(xy)
        except Exception:
            if np.linalg.matrix_rank(xy-xy[0])<2:return np.empty((0,3),np.float32),np.empty((0,3),np.uint32),0.
            raise
        simplexes=tri.find_simplex(points[finite])
        inside=simplexes>=0
        transforms=tri.transform[simplexes[inside]]
        weights=np.einsum('ijk,ik->ij',transforms[:,:2],points[finite[inside]]-transforms[:,2])
        weights=np.column_stack([weights,1-weights.sum(axis=1)])
        predicted=(grid.ravel()[ids][tri.simplices[simplexes[inside]]]*weights).sum(axis=1)
        errors=np.full(len(finite),np.inf);errors[inside]=np.abs(predicted-grid.ravel()[finite[inside]])
        worst=float(errors.max(initial=0))
        if worst<=tolerance+.001:break
        order=np.argsort(-errors);order=order[errors[order]>tolerance]
        # 1つの三角形に点を固めず、誤差が大きい三角形ごとに最大の地点を追加する。
        _,first=np.unique(simplexes[order],return_index=True)
        chosen=order[np.sort(first)[:batch_size]]
        additions=set(map(int,finite[chosen]))-selected
        if not additions:raise ValueError('誤差を減らす頂点を追加できません')
        selected.update(additions)
    else:raise ValueError('不規則メッシュの生成が収束しません')
    faces=tri.simplices.astype(np.uint32)
    if not valid.all():
        # 欠損のある格子をまたぐ面を落とす。海岸で隙間が増える場合があるため別途検証する。
        cells=valid[:-1,:-1]&valid[1:,:-1]&valid[:-1,1:]&valid[1:,1:]
        rectangles=[]
        for row in range(n):
            padded=np.pad(cells[row].astype(np.int8),(1,1));runs=np.flatnonzero(np.diff(padded))
            rectangles.extend(box(a,row,b,row+1) for a,b in zip(runs[::2],runs[1::2]))
        domain=shapely.union_all(rectangles)
        faces=faces[shapely.covered_by(shapely.polygons(xy[faces]),domain)]
    vertices=np.column_stack([xy,grid.ravel()[ids]]).astype(np.float32)
    return vertices,faces,worst


def prototype(source,reference,output,latitude=35.4,longitude=138.73,size=30,zoom=12,tolerance=25):
    from polygon_region import payload
    from overlay_region import add_overlays
    from region_server import terrain_html
    src=sqlite3.connect(source.resolve().as_uri()+'?mode=ro',uri=True)
    ref=sqlite3.connect(reference.resolve().as_uri()+'?mode=ro',uri=True)
    delta=size/111/2
    west,north=tile_position(longitude-delta/np.cos(np.radians(latitude)),latitude+delta,zoom)
    east,south=tile_position(longitude+delta/np.cos(np.radians(latitude)),latitude-delta,zoom)
    output.parent.mkdir(parents=True,exist_ok=True)
    temp=output.with_suffix('.part.sqlite3')
    if temp.exists():temp.unlink()
    dest=sqlite3.connect(temp)
    dest.execute('CREATE TABLE tiles(z INTEGER,x INTEGER,y INTEGER,data BLOB,PRIMARY KEY(z,x,y))')
    dest.execute('CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT)')
    reports=[];start=time.perf_counter()
    for x in range(int(west),int(east)+1):
        for y in range(int(north),int(south)+1):
            old=ref.execute('SELECT data FROM tiles WHERE z=? AND x=? AND y=?',(zoom,x,y)).fetchone()
            if old is None:continue
            grid=read_grid(src,zoom,x,y)
            vertices,faces,error=simplify_irregular(grid,tolerance)
            body=encode_irregular(vertices,faces)
            restored,restored_faces=decode_mesh(body)
            assert np.array_equal(vertices,restored) and np.array_equal(faces,restored_faces)
            previous,previous_faces=decode_mesh(old[0])
            reports.append({'x':x,'y':y,'oldVertices':len(previous),'newVertices':len(vertices),'oldTriangles':len(previous_faces),'newTriangles':len(faces),'oldBytes':len(old[0]),'newBytes':len(body),'maxErrorMeters':error})
            dest.execute('INSERT INTO tiles VALUES (?,?,?,?)',(zoom,x,y,body));dest.commit()
            print(len(reports),x,y,len(vertices),len(faces),flush=True)
    metadata={'schemaFormat':'yamamuki-terrain-mesh','schemaVersion':1,'encoding':'YMP3-experimental','levels':[zoom],'meshErrorBounds':{zoom:tolerance},'source':'国土地理院 DEM10B を加工。不規則 TIN の地域試作。','notes':'三角形分割の復元は試作段階。接続の SHA-256 が一致しなければ読み込みを拒否する。Android / iOS の再構成は未検証。'}
    dest.executemany('INSERT INTO metadata VALUES (?,?)',[(k,json.dumps(v,ensure_ascii=False)) for k,v in metadata.items()]);dest.commit();dest.close();src.close();ref.close();temp.replace(output)
    report={'tiles':reports,'seconds':time.perf_counter()-start,'totals':{k:sum(r[k] for r in reports) for k in ['oldVertices','newVertices','oldTriangles','newTriangles','oldBytes','newBytes']}}
    output.with_suffix('.report.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
    terrain=payload(output,latitude,longitude,size,zoom)
    terrain=add_overlays(terrain,Path('build/terrain-vectors.sqlite3'))
    terrain['experimentalTIN']=True
    output.with_suffix('.html').write_text(terrain_html(terrain),encoding='utf-8')
    print(json.dumps(report['totals']),flush=True)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,default=Path('build/terrain-lod.sqlite3'))
    parser.add_argument('--reference',type=Path,default=Path('build/terrain-mesh.sqlite3'))
    parser.add_argument('--output',type=Path,default=Path('build/terrain-tin.sqlite3'))
    parser.add_argument('--tolerance',type=float,default=25)
    args=parser.parse_args();prototype(args.source,args.reference,args.output,tolerance=args.tolerance)
