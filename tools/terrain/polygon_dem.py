"""地形の変化に応じて三角形を減らし、境界をそろえて保存する。"""
import argparse
from bisect import bisect_left, bisect_right
from concurrent.futures import ProcessPoolExecutor
import gzip
import json
import lzma
from pathlib import Path
import sqlite3
import struct
from functools import lru_cache

import numpy as np
from compact_dem import decode

HEADER=struct.Struct('<4sHfIIB')
LEVELS={14:10.,13:17.5,12:25.,11:50.,10:75.,9:112.5,8:150.,7:225.,6:300.,5:450.,4:600.}


def fan_error(values):
    n=values.shape[0]-1;m=n/2
    a,b,c,d=values[0,0],values[0,-1],values[-1,-1],values[-1,0]
    h=values[n//2,n//2]
    y,x=np.indices(values.shape,dtype=np.float32);x-=m;y-=m
    top=h+x*(b-a)/n+y*(h-(a+b)/2)/m
    right=h+x*((b+c)/2-h)/m+y*(c-b)/n
    bottom=h+x*(c-d)/n+y*((c+d)/2-h)/m
    left=h+x*(h-(a+d)/2)/m+y*(d-a)/n
    reference=np.where(np.abs(y)>=np.abs(x),np.where(y<=0,top,bottom),np.where(x>=0,right,left))
    return float(np.max(np.abs(values-reference)))


def simplify(values,tolerance,with_tree=False):
    n=values.shape[0]-1
    if values.shape!=(n+1,n+1) or n<2 or n&(n-1):
        raise ValueError('高さは 2 の累乗 + 1 の正方形が必要です')
    leaves=[];tree=[]
    def visit(x,y,size):
        block=values[y:y+size+1,x:x+size+1]
        valid=np.isfinite(block)
        if not valid.any():tree.append(0);return
        if valid.all() and (size==1 or fan_error(block)<=tolerance):
            tree.append(1);leaves.append((x,y,size));return
        if size==1:tree.append(0);return
        tree.append(2)
        half=size//2
        for dx,dy in [(0,0),(half,0),(0,half),(half,half)]:visit(x+dx,y+dy,half)
    visit(0,0,n)
    xy,indices=mesh_geometry(leaves,n)
    vertices=np.column_stack([xy,values[xy[:,1],xy[:,0]]]).astype(np.float32)
    if with_tree:return vertices,indices,np.asarray(tree,dtype=np.uint8)
    return vertices,indices


def mesh_geometry(leaves,n):
    horizontal={};vertical={}
    for x,y,size in leaves:
        for xx,yy in [(x,y),(x+size,y),(x+size,y+size),(x,y+size)]:
            horizontal.setdefault(yy,set()).add(xx);vertical.setdefault(xx,set()).add(yy)
    # 別タイルと接する辺は元の全標高点を共有し、独立した簡略化で隙間を作らない。
    for p in range(n+1):
        horizontal.setdefault(0,set()).add(p);horizontal.setdefault(n,set()).add(p)
        vertical.setdefault(0,set()).add(p);vertical.setdefault(n,set()).add(p)
    horizontal={k:sorted(v) for k,v in horizontal.items()};vertical={k:sorted(v) for k,v in vertical.items()}
    vertices=[];indices=[];lookup={}
    def vertex(x,y):
        key=(x,y)
        if key not in lookup:
            lookup[key]=len(vertices);vertices.append((x,y))
        return lookup[key]
    for x,y,size in leaves:
        if size==1:
            a,b,c,d=[vertex(xx,yy) for xx,yy in [(x,y),(x+1,y),(x+1,y+1),(x,y+1)]]
            indices.extend([(a,b,d),(b,c,d)]);continue
        top=horizontal[y];right=vertical[x+size];bottom=horizontal[y+size];left=vertical[x]
        ring=[(xx,y) for xx in top[bisect_left(top,x):bisect_left(top,x+size)]]
        ring+=[(x+size,yy) for yy in right[bisect_left(right,y):bisect_left(right,y+size)]]
        ring+=[(xx,y+size) for xx in reversed(bottom[bisect_right(bottom,x):bisect_right(bottom,x+size)])]
        ring+=[(x,yy) for yy in reversed(left[bisect_right(left,y):bisect_right(left,y+size)])]
        center=vertex(x+size//2,y+size//2)
        for i,p in enumerate(ring):indices.append((center,vertex(*p),vertex(*ring[(i+1)%len(ring)])))
    return np.asarray(vertices,dtype=np.int16).reshape(-1,2),np.asarray(indices,dtype=np.uint32).reshape(-1,3)


def encode_mesh(vertices,indices,size=257,step=5,tree=None):
    if tree is not None:
        codes=np.pad(tree,(0,(-len(tree))%4)).reshape(-1,4)
        packed=(codes*np.array([1,4,16,64],dtype=np.uint8)).sum(axis=1).astype(np.uint8).tobytes()
        q=np.rint(vertices[:,2]/step).astype(np.int32)
        delta=np.diff(q,prepend=0)
        if delta.size and (delta.min()<-32768 or delta.max()>32767):raise ValueError("標高差が保存範囲を超えています")
        raw=packed+delta.astype("<i2").tobytes()
        return HEADER.pack(b"YMP2",size,step,len(vertices),len(tree),0)+lzma.compress(raw,preset=6)
    points=vertices.copy();points[:,2]=np.rint(points[:,2]/step)
    if points.size and (points.min()<-32768 or points.max()>32767):raise ValueError('頂点の値が保存範囲を超えています')
    wide=int(len(vertices)>65535)
    raw=points.astype('<i2').tobytes()+indices.astype('<u4' if wide else '<u2').tobytes()
    return HEADER.pack(b'YMP1',size,step,len(vertices),len(indices),wide)+lzma.compress(raw,preset=6)


def encode_coast_mesh(vertices,indices,step=5):
    points=np.asarray(vertices,dtype='<f4').copy();points[:,2]=np.rint(points[:,2]/step)
    indices=np.asarray(indices,dtype=np.int64).reshape(-1,3)
    if not np.isfinite(points).all() or (indices.size and (indices.min()<0 or indices.max()>=len(points))):raise ValueError('海岸ポリゴンの頂点が不正です')
    delta=np.diff(indices.ravel(),prepend=0).astype('<i4')
    raw=points.tobytes()+delta.tobytes()
    return HEADER.pack(b'YMP4',257,step,len(points),len(indices),0)+lzma.compress(raw,preset=6)


def decode_mesh(body):
    magic,size,step,nv,nt,wide=HEADER.unpack_from(body)
    if magic==b'YMP4':
        if size!=257 or wide!=0 or nv>1000000 or nt>3000000:raise ValueError('海岸ポリゴン形式が不正です')
        raw=lzma.decompress(body[HEADER.size:]);boundary=nv*12
        if len(raw)!=boundary+nt*12:raise ValueError('海岸ポリゴンの長さが不正です')
        points=np.frombuffer(raw[:boundary],dtype='<f4').reshape(nv,3).copy();points[:,2]*=step
        indices=np.frombuffer(raw[boundary:],dtype='<i4').cumsum(dtype=np.int64)
        if not np.isfinite(points).all() or (indices.size and (indices.min()<0 or indices.max()>=nv)):raise ValueError('海岸ポリゴンの頂点が不正です')
        return points,indices.astype(np.uint32).reshape(nt,3)
    if magic==b'YMP3':
        from irregular_dem import decode_irregular
        return decode_irregular(body)
    if magic==b'YMP2':
        if size!=257 or nv>100000 or nt>100000 or wide!=0:raise ValueError('分割情報が不正です')
        raw=lzma.decompress(body[HEADER.size:]);length=(nt+3)//4
        if len(raw)!=length+nv*2:raise ValueError('分割データの長さが不正です')
        codes=((np.frombuffer(raw[:length],dtype=np.uint8)[:,None]>>np.array([0,2,4,6],dtype=np.uint8))&3).ravel()[:nt]
        leaves=[];cursor=0
        def visit(x,y,n):
            nonlocal cursor
            if cursor>=len(codes):raise ValueError('分割情報が途中で終わっています')
            code=int(codes[cursor]);cursor+=1
            if code==0:return
            if code==1:leaves.append((x,y,n));return
            if code!=2 or n<2:raise ValueError('分割情報が不正です')
            half=n//2
            for dx,dy in [(0,0),(half,0),(0,half),(half,half)]:visit(x+dx,y+dy,half)
        visit(0,0,size-1)
        if cursor!=len(codes):raise ValueError('余分な分割情報があります')
        xy,indices=mesh_geometry(leaves,size-1)
        if len(xy)!=nv:raise ValueError('頂点数が一致しません')
        heights=np.frombuffer(raw[length:],dtype='<i2').cumsum(dtype=np.int32)*step
        return np.column_stack([xy,heights]).astype(np.float32),indices
    if magic!=b'YMP1' or size!=257 or wide not in (0,1) or nv>100000 or nt>300000:raise ValueError('ポリゴン形式が不正です')
    raw=lzma.decompress(body[HEADER.size:]);boundary=nv*6
    if len(raw)!=boundary+nt*3*(4 if wide else 2):raise ValueError('ポリゴンのデータ長が不正です')
    vertices=np.frombuffer(raw[:boundary],dtype='<i2').reshape(nv,3).astype(np.float32);vertices[:,2]*=step
    indices=np.frombuffer(raw[boundary:],dtype='<u4' if wide else '<u2').reshape(nt,3).astype(np.uint32)
    if indices.size and indices.max()>=nv:raise ValueError('頂点の番号が不正です')
    return vertices,indices


@lru_cache(maxsize=2048)
def read_tile(db,z,x,y):
    row=db.execute('SELECT data FROM tiles WHERE z=? AND x=? AND y=?',(z,x,y)).fetchone()
    return None if row is None else decode(row[0])


def read_grid(db,z,x,y):
    grid=np.full((257,257),np.nan,dtype=np.float32)
    for dx,dy in [(0,0),(1,0),(0,1),(1,1)]:
        tile=read_tile(db,z,x+dx,y+dy)
        if tile is None:continue
        h=1 if dy else 256;w=1 if dx else 256
        grid[dy*256:dy*256+h,dx*256:dx*256+w]=tile[:h,:w]
    return grid


def convert(row):
    z,x,y,grid,tolerance=row
    vertices,indices,tree=simplify(grid,tolerance,True)
    body=encode_mesh(vertices,indices,tree=tree)
    restored,faces=decode_mesh(body)
    if not np.array_equal(vertices,restored) or not np.array_equal(indices,faces):raise ValueError('ポリゴンの復元が一致しません')
    return z,x,y,body,len(vertices),len(indices)


def build(source,output,workers=8,resume=False,levels=None):
    selected={z:LEVELS[z] for z in (levels if levels is not None else LEVELS)}
    if not selected:raise ValueError("生成するレベルがありません")
    src=sqlite3.connect(source.resolve().as_uri()+'?mode=ro',uri=True)
    output.parent.mkdir(parents=True,exist_ok=True)
    temp=output.with_suffix('.part.sqlite3')
    existed=temp.exists()
    if existed and not resume:raise ValueError('未完成ファイルがあります。再開する場合は --resume を指定してください')
    dest=sqlite3.connect(temp);dest.execute('PRAGMA page_size=512')
    dest.execute('CREATE TABLE IF NOT EXISTS tiles (id INTEGER PRIMARY KEY,z INTEGER,x INTEGER,y INTEGER,data BLOB)')
    dest.execute('CREATE UNIQUE INDEX IF NOT EXISTS coordinates ON tiles(z,x,y)')
    dest.execute('CREATE TABLE IF NOT EXISTS metadata (key TEXT PRIMARY KEY,value TEXT)')
    meta={'schemaFormat':'yamamuki-terrain-mesh','schemaVersion':1,'levels':list(selected),'encoding':'YMP2','source':'国土地理院 DEM10B を加工','meshErrorBounds':{z:t*2 for z,t in selected.items()},'notes':'誤差上限は入力の各縮尺の格子に対するもの。元DEMの誤差・概略平均化の誤差は別。タイル境界は同じレベルで全標高点を共有。'}
    if existed:
        old={k:json.loads(v) for k,v in dest.execute('SELECT key,value FROM metadata')}
        if any(old.get(k)!=json.loads(json.dumps(meta[k])) for k in ['schemaFormat','schemaVersion','levels','encoding','meshErrorBounds']):raise ValueError('未完成ファイルの生成条件が一致しません')
    fingerprint={'path':str(source.resolve()),'bytes':source.stat().st_size,'modifiedNs':source.stat().st_mtime_ns}
    old_fingerprint=dest.execute("SELECT value FROM metadata WHERE key='sourceFingerprint'").fetchone()
    if old_fingerprint and json.loads(old_fingerprint[0])!=fingerprint:raise ValueError('元データが変更されています')
    meta['sourceFingerprint']=fingerprint
    dest.executemany('INSERT OR REPLACE INTO metadata VALUES (?,?)',[(k,json.dumps(v,ensure_ascii=False)) for k,v in meta.items()]);dest.commit()
    reports=[]
    with ProcessPoolExecutor(max_workers=workers) as pool:
        for z,tolerance in selected.items():
            coords=src.execute('SELECT x,y FROM tiles WHERE z=? ORDER BY x,y',(z,)).fetchall()
            completed=set(dest.execute('SELECT x,y FROM tiles WHERE z=?',(z,)).fetchall())
            pending=[p for p in coords if p not in completed]
            total=0;triangles=0;vertices=0
            for start in range(0,len(pending),64):
                tasks=[(z,x,y,read_grid(src,z,x,y),tolerance) for x,y in pending[start:start+64]]
                for zz,x,y,body,nv,nt in pool.map(convert,tasks):
                    dest.execute('INSERT INTO tiles(z,x,y,data) VALUES (?,?,?,?)',(zz,x,y,body));total+=len(body);triangles+=nt;vertices+=nv
                dest.commit()
                if start%4096==0:print(z,len(completed)+start+len(tasks),'/',len(coords),flush=True)
            count,total=dest.execute('SELECT count(*),sum(length(data)) FROM tiles WHERE z=?',(z,)).fetchone()
            reports.append({'zoom':z,'tiles':count,'bytes':total});print(json.dumps(reports[-1]),flush=True)
            read_tile.cache_clear()
    assert dest.execute('PRAGMA quick_check').fetchone()[0]=='ok'
    src.close();dest.close();temp.replace(output)
    with output.open('rb') as source,gzip.open(str(output)+'.gz','wb',compresslevel=6) as target:
        import shutil
        shutil.copyfileobj(source,target)
    report={'levels':reports,'sqliteBytes':output.stat().st_size,'gzipBytes':Path(str(output)+'.gz').stat().st_size,**meta}
    output.with_suffix('.report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
    print(json.dumps(report,ensure_ascii=False),flush=True)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,default=Path('build/terrain-lod.sqlite3'))
    parser.add_argument('--output',type=Path,default=Path('build/terrain-mesh.sqlite3'))
    parser.add_argument('--workers',type=int,default=8)
    parser.add_argument('--resume',action='store_true')
    parser.add_argument('--levels',type=int,nargs='+',choices=list(LEVELS),help='指定したレベルだけ生成する')
    args=parser.parse_args();build(args.source,args.output,args.workers,args.resume,args.levels)
