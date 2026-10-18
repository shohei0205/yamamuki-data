"""国土地理院の全国河川・徒歩道・県境を取得し、地形と同じ 11 段階へ変換する。"""
import argparse
from concurrent.futures import ThreadPoolExecutor,ProcessPoolExecutor
from contextlib import closing
import gzip
import hashlib
import heapq
import json
import math
import os
from pathlib import Path
import shutil
import sqlite3
import struct
import time

import numpy as np
import shapely
from shapely.geometry import LineString,box
from ridges import RADIUS
from river_lod import line_parts,network_lines,simplify_network

WORLD=2*math.pi*RADIUS
LEVELS={4:(2000,10000),5:(1250,7500),6:(800,5000),7:(450,3500),8:(250,2000),9:(125,1000),10:(60,500),11:(25,250),12:(12,100),13:(4,30),14:(0,0)}
KINDS=('river','trail','boundary')
UNIT_ZOOM=8
HEADER=struct.Struct('<4sI')


def varint(value):
    result=bytearray()
    while value>=128:result.append((value&127)|128);value>>=7
    result.append(value);return result


def encode_lines(lines):
    # 全国共通の 1 m 格子へ丸め、タイル・レベル間で端点の座標をそろえる。
    kept=[]
    for line in lines:
        xy=np.rint(np.asarray(line.coords)[:,:2]).astype(np.int64)
        xy=xy[np.r_[True,np.any(np.diff(xy,axis=0),axis=1)]]
        if len(xy)>1:kept.append(xy)
    raw=bytearray(HEADER.pack(b'YVL1',len(kept)));px=py=0
    for xy in kept:
        raw.extend(varint(len(xy)))
        for x,y in xy:
            for d in (int(x)-px,int(y)-py):raw.extend(varint((d<<1)^(d>>63)))
            px,py=int(x),int(y)
    return gzip.compress(raw,mtime=0),len(kept),sum(len(xy) for xy in kept)


def decode_lines(body):
    raw=gzip.decompress(body)
    if len(raw)<HEADER.size:raise ValueError('河川・徒歩道・県境データのヘッダーがありません')
    magic,count=HEADER.unpack_from(raw)
    if magic!=b'YVL1' or count>10_000_000:raise ValueError('線データの形式が不正です')
    position=HEADER.size
    def read():
        nonlocal position
        value=0
        for shift in range(0,70,7):
            if position>=len(raw):raise ValueError('線データが途中で終わっています')
            byte=raw[position];position+=1;value|=(byte&127)<<shift
            if not byte&128:return value
        raise ValueError('線データの整数が不正です')
    result=[];px=py=0
    for _ in range(count):
        size=read()
        if size<2 or size>len(raw)//2:raise ValueError('線の頂点数が不正です')
        xy=[]
        for i in range(size):
            dx=read();dy=read();px+=(dx>>1)^-(dx&1);py+=(dy>>1)^-(dy&1)
            if abs(px)>WORLD or abs(py)>WORLD:raise ValueError('線の座標が範囲外です')
            xy.append((px,py))
        result.append(np.asarray(xy,dtype=float))
    if position!=len(raw):raise ValueError('線データの末尾に余分な値があります')
    return result


def source_index(reader,snapshot,path,zoom=14):
    from pmtiles.tile import deserialize_directory,tileid_to_zxy
    if path.exists():
        value=json.loads(path.read_text(encoding='utf-8'))
        if value['snapshot']!=snapshot:raise ValueError('取得元の更新時点が異なります')
        return value['tiles']
    header=reader.header();low=(4**zoom-1)//3;high=(4**(zoom+1)-1)//3;result=[]
    leaf_start=header['leaf_directory_offset'];leaf_end=leaf_start+header['leaf_directory_length']
    leaf_data=reader.get_bytes(leaf_start,header['leaf_directory_length'])
    def directory(offset,length):
        return leaf_data[offset-leaf_start:offset-leaf_start+length] if leaf_start<=offset and offset+length<=leaf_end else reader.get_bytes(offset,length)
    def visit(offset,length,end):
        entries=deserialize_directory(directory(offset,length))
        for i,e in enumerate(entries):
            upper=entries[i+1].tile_id if i+1<len(entries) else end
            if e.tile_id>=high or upper<=low:continue
            if not e.run_length:visit(header['leaf_directory_offset']+e.offset,e.length,upper)
            else:
                for tid in range(max(low,e.tile_id),min(high,e.tile_id+e.run_length)):
                    z,x,y=tileid_to_zxy(tid);result.append([x,y,header['tile_data_offset']+e.offset,e.length])
    visit(header['root_offset'],header['root_length'],high)
    path.write_text(json.dumps({'snapshot':snapshot,'tiles':result},separators=(',',':')),encoding='utf-8')
    return result


def batches(tiles,limit=8*1024*1024):
    result=[];current=[];start=end=0
    for row in sorted(tiles,key=lambda row:row[2]):
        offset,length=row[2:]
        if current and offset+length-start>limit:
            result.append((start,end-start,current));current=[]
        if not current:start=offset
        end=max(end,offset+length) if current else offset+length;current.append(row)
    if current:result.append((start,end-start,current))
    return result


def extract_batch(task):
    from mapbox_vector_tile.decoder import TileData
    offset,length,tiles,cache=task
    body=(Path(cache)/f'{offset}-{length}.range').read_bytes();result=[]
    for x,y,start,size in tiles:
        chunk=body[start-offset:start-offset+size]
        raw=gzip.decompress(chunk) if chunk[:2]==b'\x1f\x8b' else chunk
        decoder=TileData(raw,default_options={'y_coord_down':True})
        for i in reversed(range(len(decoder.tile.layers))):
            if decoder.tile.layers[i].name not in ('RvrCL','RdCL','AdmBdry'):del decoder.tile.layers[i]
        result.append((x,y,gzip.compress(decoder.tile.SerializeToString(),mtime=0)))
    return result


def write_progress(root,stage,done,total,started,**kwargs):
    value={'stage':stage,'done':done,'total':total,'elapsedSeconds':round(time.monotonic()-started,1),**kwargs}
    temp=root/f'progress.{os.getpid()}.{time.time_ns()}.tmp.json'
    try:
        temp.write_text(json.dumps(value,ensure_ascii=False,indent=2),encoding='utf-8')
        for attempt in range(5):
            try:temp.replace(root/'progress.json');break
            except PermissionError:
                if attempt==4:raise
                time.sleep(.05*2**attempt)
    except PermissionError as error:
        # 進捗ファイルの一時的なロックで、確定済みデータの生成を中断しない。
        value['progressFileError']=str(error)
        try:temp.unlink(missing_ok=True)
        except PermissionError:pass
    print(json.dumps(value,ensure_ascii=False),flush=True)


def acquire(root,workers=4):
    from coastline_source import remote_reader
    root.mkdir(parents=True,exist_ok=True);cache=root/'source-ranges'
    reader,snapshot=remote_reader(cache)
    tiles=source_index(reader,snapshot,root/'source-index14.json');groups=batches(tiles)
    started=time.monotonic()
    def fetch(group):
        offset,length,_=group
        if shutil.disk_usage(root).free<4*1024**3:raise ValueError('空き容量が 4 GB 未満になったため取得を中断しました。空きを確保して再開してください')
        getattr(reader.get_bytes,'__wrapped__',reader.get_bytes)(offset,length);return length
    with ThreadPoolExecutor(max_workers=workers) as pool:
        amount=0
        for i,length in enumerate(pool.map(fetch,groups)):
            amount+=length
            if i%8==0 or i+1==len(groups):write_progress(root,'download',i+1,len(groups),started,bytes=amount)
    with closing(sqlite3.connect(root/'source.sqlite3')) as db:
        db.execute('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT)')
        db.execute('CREATE TABLE IF NOT EXISTS tiles(x INTEGER,y INTEGER,body BLOB,PRIMARY KEY(x,y)) WITHOUT ROWID')
        stored=db.execute("SELECT value FROM metadata WHERE key='sourceSnapshot'").fetchone()
        if stored and json.loads(stored[0])!=snapshot:raise ValueError('抽出済みデータの取得元が異なります')
        db.execute('INSERT OR REPLACE INTO metadata VALUES (?,?)',('sourceSnapshot',json.dumps(snapshot)));db.commit()
        saved=set(db.execute('SELECT x,y FROM tiles'))
        pending=[(o,n,[t for t in rows if (t[0],t[1]) not in saved],str(cache)) for o,n,rows in groups]
        pending=[task for task in pending if task[2]]
        done=len(saved)
        with ProcessPoolExecutor(max_workers=workers) as pool:
            for rows in pool.map(extract_batch,pending):
                db.executemany('INSERT OR REPLACE INTO tiles VALUES (?,?,?)',rows);db.commit();done+=len(rows)
                write_progress(root,'extract',done,len(tiles),started)
        db.executemany('INSERT OR REPLACE INTO metadata VALUES (?,?)',[(k,json.dumps(v)) for k,v in {'sourceZoom':14,'sourceTileCount':len(tiles),'complete':True,'sourceUrl':'https://cyberjapandata.gsi.go.jp/xyz/optimal_bvmap-v1/optimal_bvmap-v1.pmtiles'}.items()]);db.commit()
        assert db.execute('SELECT count(*) FROM tiles').fetchone()[0]==len(tiles)
        assert db.execute('PRAGMA quick_check').fetchone()[0]=='ok'
    return root/'source.sqlite3'


def prune_network(lines,minimum,coverage):
    """短い末端を除き、分岐がなくなった点をつなぎ直して長い本流を残す。"""
    if not minimum:return lines
    edges={};nodes={};queue=[];serial=0
    boundary=coverage.boundary
    def add(coords,length,protected):
        nonlocal serial
        serial+=1;a=tuple(coords[0]);b=tuple(coords[-1]);edges[serial]=(a,b,coords,length,protected)
        nodes.setdefault(a,set()).add(serial);nodes.setdefault(b,set()).add(serial)
        if length<minimum:heapq.heappush(queue,(length,serial))
        return serial
    def remove(e):
        a,b,*_=edges.pop(e);nodes[a].discard(e);nodes[b].discard(e);return a,b
    def degree(n):return sum(2 if edges[e][0]==edges[e][1] else 1 for e in nodes[n])
    for line in lines:
        xy=np.asarray(line.coords);protected=any(boundary.distance(shapely.Point(end))<.1 for end in (xy[0],xy[-1]))
        add(xy,line.length,protected)
    while queue:
        _,e=heapq.heappop(queue)
        if e not in edges:continue
        a,b,xy,length,protected=edges[e]
        if protected or length>=minimum:continue
        if a!=b and degree(a)!=1 and degree(b)!=1:continue
        affected=remove(e)
        for n in affected:
            incident=list(nodes[n])
            if len(incident)==2 and degree(n)==2:
                first,second=incident
                a,b,xy1,l1,p1=edges[first];c,d,xy2,l2,p2=edges[second]
                if tuple(xy1[-1])!=n:xy1=xy1[::-1]
                if tuple(xy2[0])!=n:xy2=xy2[::-1]
                remove(first);remove(second)
                joined=add(np.concatenate([xy1,xy2[1:]]),l1+l2,p1 or p2)
                for end in (edges[joined][0],edges[joined][1]):
                    for pending in nodes[end]:
                        if edges[pending][3]<minimum:heapq.heappush(queue,(edges[pending][3],pending))
            else:
                for pending in incident:
                    if edges[pending][3]<minimum:heapq.heappush(queue,(edges[pending][3],pending))
    return [LineString(xy) for a,b,xy,length,protected in edges.values()]


def build_unit(task):
    from mapbox_vector_tile.decoder import TileData
    from vector_layers import category
    source,x,y=task[:3];selected_levels=set(task[3]) if len(task)>3 else set(LEVELS);factor=2**(14-UNIT_ZOOM);unit=WORLD/2**UNIT_ZOOM
    boundary=box(x*unit-WORLD/2,WORLD/2-(y+1)*unit,(x+1)*unit-WORLD/2,WORLD/2-y*unit)
    groups={kind:[] for kind in KINDS};original={kind:0 for kind in KINDS}
    with closing(sqlite3.connect(Path(source).resolve().as_uri()+'?mode=ro',uri=True)) as db:
        rows=db.execute('SELECT x,y,body FROM tiles WHERE x>=? AND x<? AND y>=? AND y<?',(x*factor,(x+1)*factor,y*factor,(y+1)*factor)).fetchall()
    from shapely.geometry import shape
    size=WORLD/2**14
    for tx,ty,body in rows:
        decoded=TileData(gzip.decompress(body),default_options={'y_coord_down':True}).get_message()
        for name,layer in decoded.items():
            extent=layer.get('extent',4096)
            for item in layer.get('features',[]):
                kind=category(name,item['properties'])
                if kind not in groups:continue
                geometry=shape(item['geometry'])
                geometry=shapely.transform(geometry,lambda points:np.column_stack([(tx+points[:,0]/extent)*size-WORLD/2,WORLD/2-(ty+points[:,1]/extent)*size]))
                # 北海道には他県との陸上県境がない。1211 に混在する振興局界を除く。
                if kind=='boundary':geometry=geometry.intersection(box(-WORLD, -WORLD,WORLD,RADIUS*math.log(math.tan(math.pi/4+math.radians(41.3)/2))))
                geometry=geometry.intersection(boundary)
                for line in line_parts(geometry):
                    if line.length:groups[kind].append(line);original[kind]+=len(line.coords)
    result=[]
    for kind,geometries in groups.items():
        if not geometries:continue
        network=network_lines(geometries)
        pruned=network
        for level,(tolerance,minimum) in reversed(list(LEVELS.items())):
            pruned=pruned if kind=='boundary' else prune_network(pruned,minimum,boundary)
            if level not in selected_levels:continue
            selected=[line.simplify(tolerance,preserve_topology=True) if tolerance else line for line in pruned]
            body,count,vertices=encode_lines(selected)
            if count:result.append((level,kind,x,y,body,count,vertices))
    return x,y,result,original


def generate(source,output,root,workers=4):
    root.mkdir(parents=True,exist_ok=True);started=time.monotonic();temp=output.with_suffix('.part.sqlite3')
    with closing(sqlite3.connect(source.resolve().as_uri()+'?mode=ro',uri=True)) as src:
        meta={k:json.loads(v) for k,v in src.execute('SELECT key,value FROM metadata')}
        if not meta.get('complete'):raise ValueError('全国の元データの取得が未完了です')
        units=src.execute('SELECT DISTINCT x/64,y/64 FROM tiles ORDER BY x/64,y/64').fetchall()
    output.parent.mkdir(parents=True,exist_ok=True)
    with closing(sqlite3.connect(temp,timeout=60)) as db:
        db.execute('PRAGMA journal_mode=WAL')
        db.execute('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT)')
        db.execute('CREATE TABLE IF NOT EXISTS line_tiles(id INTEGER PRIMARY KEY,level INTEGER,kind TEXT,x INTEGER,y INTEGER,body BLOB,lines INTEGER,vertices INTEGER,UNIQUE(level,kind,x,y))')
        db.execute('CREATE INDEX IF NOT EXISTS line_level ON line_tiles(level,kind)')
        db.execute('CREATE VIRTUAL TABLE IF NOT EXISTS line_bounds USING rtree(id,west,east,south,north)')
        db.execute('CREATE TABLE IF NOT EXISTS completed(x INTEGER,y INTEGER,PRIMARY KEY(x,y)) WITHOUT ROWID')
        settings={'levels':LEVELS,'unitZoom':UNIT_ZOOM,'encoding':'YVL1','algorithm':'shortest-leaf-pruning-v1','hokkaidoBoundaryCutoffLatitude':41.3}
        previous=db.execute("SELECT value FROM metadata WHERE key='generationSettings'").fetchone()
        if previous and json.loads(previous[0])!=json.loads(json.dumps(settings)):raise ValueError('変換途中の生成設定が異なります。別の出力ファイルを指定してください')
        db.execute('INSERT OR REPLACE INTO metadata VALUES (?,?)',('generationSettings',json.dumps(settings)))
        stored=db.execute("SELECT value FROM metadata WHERE key='sourceSnapshot'").fetchone()
        if stored and json.loads(stored[0])!=meta['sourceSnapshot']:raise ValueError('変換途中の取得元が異なります')
        db.execute('INSERT OR REPLACE INTO metadata VALUES (?,?)',('sourceSnapshot',json.dumps(meta['sourceSnapshot'])));db.commit()
        saved=set(db.execute('SELECT x,y FROM completed'));tasks=[(str(source),x,y) for x,y in units if (x,y) not in saved]
        done=len(saved)
        with ProcessPoolExecutor(max_workers=workers) as pool:
            for x,y,rows,original in pool.map(build_unit,tasks,chunksize=1):
                for level,kind,tx,ty,body,count,vertices in rows:
                    cursor=db.execute('INSERT INTO line_tiles(level,kind,x,y,body,lines,vertices) VALUES (?,?,?,?,?,?,?)',(level,kind,tx,ty,body,count,vertices))
                    xy=[point for line in decode_lines(body) for point in line];coords=np.asarray(xy)
                    west,east=np.degrees(coords[:,0].min()/RADIUS),np.degrees(coords[:,0].max()/RADIUS)
                    south,north=np.degrees(2*np.arctan(np.exp(np.array([coords[:,1].min(),coords[:,1].max()])/RADIUS))-math.pi/2)
                    db.execute('INSERT INTO line_bounds VALUES (?,?,?,?,?)',(cursor.lastrowid,west,east,south,north))
                db.execute('INSERT INTO completed VALUES (?,?)',(x,y));db.commit();done+=1
                write_progress(root,'generate',done,len(units),started,unit=[x,y])
        reports=[{'level':z,'kind':k,'tiles':n,'lines':lines,'vertices':vertices,'bodyBytes':size} for z,k,n,lines,vertices,size in db.execute('SELECT level,kind,count(*),sum(lines),sum(vertices),sum(length(body)) FROM line_tiles GROUP BY level,kind ORDER BY level,kind')]
        values={'schemaFormat':'yamamuki-national-lines','schemaVersion':1,'encoding':'YVL1-gzip','coordinateStepMeters':1,'unitZoom':UNIT_ZOOM,'levels':list(LEVELS),'layerReports':reports,'bounds':[122,17,155,46],'attribution':'国土地理院最適化ベクトルタイルを加工して作成','sourceSnapshot':meta['sourceSnapshot'],'sourceTileCount':meta['sourceTileCount'],'national':True,'complete':True,'sourceUrl':meta['sourceUrl'],'notes':'徒歩道は RdCL 2721。河川の大小は短い末端の枝を省く近似。北海道の振興局界は除外。1 m Web Mercator 格子で端点を共通化。'}
        db.executemany('INSERT OR REPLACE INTO metadata VALUES (?,?)',[(k,json.dumps(v,ensure_ascii=False)) for k,v in values.items()]);db.commit()
        assert db.execute('PRAGMA quick_check').fetchone()[0]=='ok'
        db.execute('PRAGMA wal_checkpoint(TRUNCATE)')
        db.execute('PRAGMA journal_mode=DELETE')
    temp.replace(output)
    packed=output.with_suffix(output.suffix+'.gz')
    with output.open('rb') as src,packed.open('wb') as dst,gzip.GzipFile(fileobj=dst,mode='wb',mtime=0) as gz:shutil.copyfileobj(src,gz)
    report={'sqliteBytes':output.stat().st_size,'gzipBytes':packed.stat().st_size,'levels':reports}
    output.with_suffix('.report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
    write_progress(root,'complete',len(units),len(units),started,**report)
    return report


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root',type=Path,default=Path('build/terrain-national-lines'))
    parser.add_argument('--output',type=Path,default=Path('build/terrain-national-lines.sqlite3'))
    parser.add_argument('--workers',type=int,default=4)
    parser.add_argument('--stage',choices=['all','acquire','generate'],default='all')
    args=parser.parse_args()
    if args.stage in ('all','acquire'):acquire(args.root,args.workers)
    if args.stage in ('all','generate'):generate(args.root/'source.sqlite3',args.output,args.root,args.workers)
