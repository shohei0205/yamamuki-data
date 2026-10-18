"""国土地理院の海域ポリゴンから、DEM 格子に依存しない陸地範囲を保存する。"""
import argparse
from concurrent.futures import ThreadPoolExecutor
from contextlib import closing
from functools import lru_cache
import gzip
import hashlib
import json
import math
from pathlib import Path
import sqlite3
import threading
import time
import urllib.request
import numpy as np
import shapely
from shapely.geometry import shape,box
from pmtiles.reader import Reader
from mapbox_vector_tile.decoder import TileData
from vector_layers import SOURCE
from ridges import RADIUS
WORLD=2*math.pi*RADIUS


def remote_reader(cache):
    cache.mkdir(parents=True,exist_ok=True)
    with urllib.request.urlopen(urllib.request.Request(SOURCE,headers={'Range':'bytes=0-126'}),timeout=45) as response:
        if response.status!=206:raise ValueError('部分取得に対応していません')
        header=response.read();snapshot={'headerSha256':hashlib.sha256(header).hexdigest(),'lastModified':response.headers.get('Last-Modified'),'etag':response.headers.get('ETag')}
    identity=cache/'snapshot.json'
    if identity.exists() and json.loads(identity.read_text())!=snapshot:raise ValueError('配信元が更新されています。別のキャッシュを指定してください')
    identity.write_text(json.dumps(snapshot,indent=2))
    @lru_cache(maxsize=512)
    def get_bytes(offset,length):
        target=cache/f'{offset}-{length}.range'
        if target.exists() and target.stat().st_size==length:return target.read_bytes()
        for attempt in range(4):
            try:
                headers={'Range':f'bytes={offset}-{offset+length-1}'}
                if snapshot['lastModified']:headers['If-Unmodified-Since']=snapshot['lastModified']
                with urllib.request.urlopen(urllib.request.Request(SOURCE,headers=headers),timeout=45) as response:
                    if response.status!=206 or not response.headers.get('Content-Range','').startswith(f'bytes {offset}-'):raise ValueError('取得位置が違います')
                    data=response.read()
                if len(data)!=length:raise ValueError('取得長が違います')
                temporary=target.with_suffix(f'.{threading.get_ident()}.tmp');temporary.write_bytes(data);temporary.replace(target)
                return data
            except Exception:
                if attempt==3:raise
                time.sleep(attempt+1)
    return Reader(get_bytes),snapshot


def tile_land(body,x,y,z=10):
    size=WORLD/2**z;left=x*size-WORLD/2;top=WORLD/2-y*size
    boundary=box(left,top-size,left+size,top)
    raw=gzip.decompress(body) if body[:2]==b'\x1f\x8b' else body
    decoder=TileData(raw,default_options={'y_coord_down':True})
    for index in reversed(range(len(decoder.tile.layers))):
        if decoder.tile.layers[index].name!='WA':del decoder.tile.layers[index]
    layer=decoder.get_message().get('WA',{});extent=layer.get('extent',4096)
    seas=[]
    for feature in layer.get('features',[]):
        if not 5100<=feature['properties'].get('vt_code',0)<5200:continue
        geometry=shape(feature['geometry'])
        geometry=shapely.transform(geometry,lambda points:np.column_stack([left+points[:,0]/extent*size,top-points[:,1]/extent*size]))
        if not geometry.is_valid:geometry=shapely.make_valid(geometry)
        seas.append(geometry.intersection(boundary))
    land=boundary.difference(shapely.union_all(seas)) if seas else boundary
    return land


def acquire(dem,output,cache):
    reader,snapshot=remote_reader(cache)
    with closing(sqlite3.connect(dem.resolve().as_uri()+'?mode=ro',uri=True)) as source:
        coords=source.execute('SELECT x,y FROM tiles WHERE z=10 ORDER BY x,y').fetchall()
    temporary=output.with_suffix('.part.sqlite3')
    if temporary.exists():raise ValueError('未完成ファイルがあります')
    with closing(sqlite3.connect(temporary)) as db:
        db.execute('CREATE TABLE tiles(x INTEGER,y INTEGER,land BLOB,PRIMARY KEY(x,y)) WITHOUT ROWID')
        db.execute('CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT)')
        def fetch(coord):
            x,y=coord;body=reader.get(10,x,y)
            if body is None:raise ValueError(f'DEM がある範囲の海岸データを取得できません: {x}/{y}')
            land=tile_land(body,x,y)
            return x,y,shapely.to_wkb(land)
        with ThreadPoolExecutor(max_workers=4) as pool:
            for i,(x,y,data) in enumerate(pool.map(fetch,coords)):
                db.execute('INSERT INTO tiles VALUES (?,?,?)',(x,y,data))
                if i%32==0:db.commit();print(i+1,'/',len(coords),flush=True)
        meta={'schemaFormat':'yamamuki-coastline-source','sourceZoom':10,'sourceUrl':SOURCE,'sourceSnapshot':snapshot,'attribution':'国土地理院最適化ベクトルタイルを加工して作成','classification':'WA vt_code 5100〜5199 の海域を除いた陸域。湖・河川は除外しない。','tileCount':len(coords)}
        db.executemany('INSERT INTO metadata VALUES (?,?)',[(k,json.dumps(v,ensure_ascii=False)) for k,v in meta.items()]);db.commit()
        assert db.execute('PRAGMA quick_check').fetchone()[0]=='ok'
    temporary.replace(output);print(output.stat().st_size,flush=True)

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--dem',type=Path,default=Path('build/terrain-mesh.sqlite3'))
    parser.add_argument('--output',type=Path,default=Path('build/terrain-coast-source.sqlite3'))
    parser.add_argument('--cache',type=Path,default=Path('build/terrain-coast-source-ranges'))
    args=parser.parse_args();acquire(args.dem,args.output,args.cache)
