"""国土地理院 PMTiles の必要範囲を保存し、水域・徒歩道・県境を抽出する。"""
import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
from functools import lru_cache
import gzip
import hashlib
import json
import math
from pathlib import Path
import sqlite3
import time
import urllib.request

from ridges import tile_position, lon_lat

SOURCE='https://cyberjapandata.gsi.go.jp/xyz/optimal_bvmap-v1/optimal_bvmap-v1.pmtiles'


def category(layer, properties):
    code=properties.get('vt_code')
    if layer=='WA':return 'water'
    if layer=='WL' and code in (5231,5232,5233,5241,5242):return 'lakeedge'
    if layer=='RvrCL' and code not in (5302,5322):return 'river'
    if layer=='RdCL' and code in (2721,2722,2723,2724,2731,2732,2733,2734):return 'trail'
    if layer=='AdmBdry' and code==1211:return 'boundary'
    return None


def geographic_geometry(geometry,x,y,z,extent):
    def walk(value):
        if value and isinstance(value[0],(int,float)):
            return list(lon_lat((x+value[0]/extent)*256,(y+value[1]/extent)*256,z))
        return [walk(v) for v in value]
    return {'type':geometry['type'],'coordinates':walk(geometry['coordinates'])}


def acquire(bounds,zoom,output,cache):
    from pmtiles.reader import Reader
    import mapbox_vector_tile
    from shapely.geometry import shape, mapping, box
    cache.mkdir(parents=True,exist_ok=True)
    # 同じオフセットのキャッシュを、更新後の別の PMTiles と混ぜない。
    request=urllib.request.Request(SOURCE,headers={'Range':'bytes=0-126'})
    with urllib.request.urlopen(request,timeout=45) as response:
        if response.status!=206:raise ValueError('部分取得に対応していません')
        header=response.read()
        snapshot={'headerSha256':hashlib.sha256(header).hexdigest(),'etag':response.headers.get('ETag'),'lastModified':response.headers.get('Last-Modified')}
    identity=cache/'snapshot.json'
    cached_header=cache/'0-127.range'
    if cached_header.exists() and cached_header.read_bytes()!=header:raise ValueError('配信元のヘッダーが変わっています。新しい --cache フォルダーを指定してください')
    if identity.exists() and json.loads(identity.read_text('utf-8'))!=snapshot:raise ValueError('配信元が更新されています。新しい --cache フォルダーを指定してください')
    identity.write_text(json.dumps(snapshot,indent=2),encoding='utf-8')
    @lru_cache(maxsize=512)
    def get_bytes(offset,length):
        target=cache/f'{offset}-{length}.range'
        if target.exists():
            cached=target.read_bytes()
            if len(cached)==length:return cached
        for attempt in range(4):
            try:
                headers={'Range':f'bytes={offset}-{offset+length-1}'}
                if snapshot['lastModified']:headers['If-Unmodified-Since']=snapshot['lastModified']
                request=urllib.request.Request(SOURCE,headers=headers)
                with urllib.request.urlopen(request,timeout=45) as response:
                    if response.status!=206:raise ValueError('部分取得に対応していません')
                    if not response.headers.get('Content-Range','').startswith(f'bytes {offset}-'):raise ValueError('取得位置が違います')
                    body=response.read()
                if len(body)!=length:raise ValueError('取得長が違います')
                target.write_bytes(body);return body
            except Exception:
                if attempt==3:raise
                time.sleep(attempt+1)
    reader=Reader(get_bytes)
    west,south,east,north=bounds
    x0,y0=map(math.floor,tile_position(west,north,zoom));x1,y1=map(math.floor,tile_position(east,south,zoom))
    coords=[(x,y) for x in range(x0,x1+1) for y in range(y0,y1+1)]
    clip=box(*bounds)
    def tile(coord,z=zoom,only_edges=False):
        x,y=coord;body=reader.get(z,x,y)
        if body is None:return []
        raw=gzip.decompress(body) if body[:2]==b'\x1f\x8b' else body
        decoded=mapbox_vector_tile.decode(raw,default_options={'y_coord_down':True})
        result=[]
        for layer in (['WL'] if only_edges else ['WA','WL','RvrCL','RdCL','AdmBdry']):
            entry=decoded.get(layer,{});extent=entry.get('extent',4096)
            for f in entry.get('features',[]):
                kind=category(layer,f['properties'])
                if not kind:continue
                geometry=shape(geographic_geometry(f['geometry'],x,y,z,extent))
                if not geometry.is_valid:geometry=geometry.buffer(0) if kind=='water' else geometry
                geometry=geometry.intersection(clip)
                if geometry.is_empty:continue
                result.append((kind,geometry.bounds,mapping(geometry),f['properties']))
        return result
    output.parent.mkdir(parents=True,exist_ok=True)
    temp=output.with_suffix('.part.sqlite3')
    if temp.exists():temp.unlink()
    db=sqlite3.connect(temp)
    db.execute('CREATE TABLE features(id INTEGER PRIMARY KEY,kind TEXT,west REAL,south REAL,east REAL,north REAL,body BLOB)')
    db.execute('CREATE INDEX feature_kind ON features(kind)')
    db.execute('CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT)')
    seen=set();lake_parents=set()
    with ThreadPoolExecutor(max_workers=4) as pool:
        for i,rows in enumerate(pool.map(tile,coords)):
            for kind,bbox,geometry,props in rows:
                if kind=='water' and props.get('vt_code')==5200 and shape(geometry).area>.00001:
                    lake_parents.add(coords[i])
                raw=json.dumps({'geometry':geometry,'properties':props},ensure_ascii=False,separators=(',',':')).encode()
                fingerprint=hashlib.sha256(kind.encode()+raw).digest()
                if fingerprint in seen:continue
                seen.add(fingerprint)
                db.execute('INSERT INTO features(kind,west,south,east,north,body) VALUES (?,?,?,?,?,?)',(kind,*bbox,gzip.compress(raw,mtime=0)))
            if i%32==0:db.commit();print(i+1,'/',len(coords),flush=True)
        # 湖池の水涯線はズーム16のみ。面積は取得範囲を絞るためだけに使い、湖の判定には使わない。
        if zoom==14:
            edges=[(x*4+dx,y*4+dy) for x,y in sorted(lake_parents) for dx in range(4) for dy in range(4)]
            for i,rows in enumerate(pool.map(lambda coord:tile(coord,16,True),edges)):
                for kind,bbox,geometry,props in rows:
                    raw=json.dumps({'geometry':geometry,'properties':props},ensure_ascii=False,separators=(',',':')).encode()
                    fingerprint=hashlib.sha256(kind.encode()+raw).digest()
                    if fingerprint in seen:continue
                    seen.add(fingerprint)
                    db.execute('INSERT INTO features(kind,west,south,east,north,body) VALUES (?,?,?,?,?,?)',(kind,*bbox,gzip.compress(raw,mtime=0)))
                if i%64==0:db.commit();print('lake edges',i+1,'/',len(edges),flush=True)
    metadata={'schemaFormat':'yamamuki-terrain-vector','schemaVersion':1,'bounds':bounds,'sourceZoom':zoom,'sourceUrl':SOURCE,'attribution':'国土地理院最適化ベクトルタイルを加工して作成','retrievedAt':datetime.now(timezone.utc).isoformat(),'sourceMetadata':reader.metadata(),'classification':{'trail':'RdCL vt_code=2721（徒歩道。登山道以外の徒歩道も含む）','boundary':'AdmBdry vt_code=1211（北海道では振興局界も含む）'}}
    metadata['sourceSnapshot']=snapshot
    db.executemany('INSERT INTO metadata VALUES (?,?)',[(k,json.dumps(v,ensure_ascii=False)) for k,v in metadata.items()]);db.commit()
    print(db.execute('SELECT kind,count(*) FROM features GROUP BY kind').fetchall(),flush=True)
    assert db.execute('PRAGMA quick_check').fetchone()[0]=='ok'
    db.close();temp.replace(output)
    print(output.stat().st_size,flush=True)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bounds',type=float,nargs=4,default=[138.45,35.0,139.,35.64])
    parser.add_argument('--zoom',type=int,default=14)
    parser.add_argument('--output',type=Path,default=Path('build/terrain-vectors.sqlite3'))
    parser.add_argument('--cache',type=Path,default=Path('build/terrain-vector-source/ranges'))
    args=parser.parse_args();acquire(args.bounds,args.zoom,args.output,args.cache)
