"""SQLite の行を加工せずブラウザーへ中継する。地形処理は行わない。"""
from contextlib import closing
import json
import math
from pathlib import Path
import sqlite3
import struct


def connect(path):
    return closing(sqlite3.connect(path.resolve().as_uri()+'?mode=ro',uri=True))


def metadata(path):
    if not path or not path.exists():return None
    with connect(path) as db:
        return {key:json.loads(value) for key,value in db.execute('SELECT key,value FROM metadata')}


def pack_rows(rows):
    descriptors=[];bodies=[]
    for properties,body in rows:
        descriptors.append({**properties,'bytes':len(body)});bodies.append(body)
    header=json.dumps(descriptors,separators=(',',':'),ensure_ascii=False).encode('utf-8')
    return struct.pack('<4sI',b'YSQ1',len(header))+header+b''.join(bodies)


def bounds(query):
    values=[float(query[key][0]) for key in ('west','south','east','north')]
    west,south,east,north=values
    if not all(map(math.isfinite,values)) or not -180<=west<east<=180 or not -85<=south<north<=85:
        raise ValueError('取得範囲が不正です')
    return west,south,east,north


def relay(path,query,database,root):
    """返却する BLOB は SQLite 内の値そのもの。圧縮・展開・再構成はしない。"""
    lines=root/'build/terrain-national-lines.sqlite3'
    water=root/'build/terrain-vectors.sqlite3'
    outline=database.with_name('terrain-outlines.sqlite3')
    if path=='/sqlite/metadata':
        outline_meta=metadata(outline)
        stat=database.stat()
        if outline_meta and outline_meta.get('sourceFingerprint')!={'bytes':stat.st_size,'modifiedNs':stat.st_mtime_ns}:outline_meta=None
        value={'terrain':metadata(database),'lines':metadata(lines),'water':metadata(water),'outline':outline_meta,'normals':metadata(root/'build/terrain-normalmaps.sqlite3')}
        return json.dumps(value,ensure_ascii=False).encode('utf-8'),'application/json; charset=utf-8'
    if path=='/sqlite/overview':
        level=int(query.get('level',['4'])[0])
        if level not in (4,5,6,7,8):raise ValueError('全国表示のレベルが不正です')
        with connect(database) as db:
            rows=[({'z':z,'x':x,'y':y},body) for z,x,y,body in db.execute('SELECT z,x,y,data FROM tiles WHERE z=? ORDER BY x,y',(level,))]
        return pack_rows(rows),'application/octet-stream'
    if path in ('/sqlite/mesh','/sqlite/outline'):
        tiles=[tuple(map(int,item.split(','))) for item in query.get('tiles',[''])[0].split(';')]
        if not 1<=len(tiles)<=256 or any(len(tile)!=3 or not 4<=tile[0]<=14 or not 0<=tile[1]<2**tile[0] or not 0<=tile[2]<2**tile[0] for tile in tiles):
            raise ValueError('タイルの指定が不正です')
        source=outline if path.endswith('outline') else database
        rows=[]
        if source.exists():
            with connect(source) as db:
                for z,x,y in dict.fromkeys(tiles):
                    row=db.execute('SELECT data FROM tiles WHERE z=? AND x=? AND y=?',(z,x,y)).fetchone()
                    if row:rows.append(({'z':z,'x':x,'y':y},row[0]))
        return pack_rows(rows),'application/octet-stream'
    if path=='/sqlite/normals':
        source=root/'build/terrain-normalmaps.sqlite3';rows=[]
        if source.exists():
            with connect(source) as db:
                rows=[({'z':z,'x':x,'y':y,'width':width,'height':height},body) for z,x,y,width,height,body in db.execute('SELECT z,x,y,width,height,data FROM tiles ORDER BY z,x,y')]
        return pack_rows(rows),'application/octet-stream'
    west,south,east,north=bounds(query)
    if path=='/sqlite/lines':
        level=int(query['level'][0])
        if not 4<=level<=14:raise ValueError('線のレベルが不正です')
        rows=[]
        if lines.exists():
            with connect(lines) as db:
                for kind,x,y,body in db.execute('SELECT t.kind,t.x,t.y,t.body FROM line_tiles t JOIN line_bounds b ON b.id=t.id WHERE t.level=? AND b.west<=? AND b.east>=? AND b.south<=? AND b.north>=?',(level,east,west,north,south)):
                    rows.append(({'kind':kind,'x':x,'y':y,'level':level},body))
        return pack_rows(rows),'application/octet-stream'
    if path=='/sqlite/water':
        rows=[]
        if water.exists():
            with connect(water) as db:
                for kind,body in db.execute("SELECT kind,body FROM features WHERE kind IN ('water','lakeedge') AND west<=? AND east>=? AND south<=? AND north>=?",(east,west,north,south)):
                    rows.append(({'kind':kind},body))
        return pack_rows(rows),'application/octet-stream'
    raise ValueError('中継先がありません')
