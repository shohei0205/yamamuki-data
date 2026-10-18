"""レベル 17 相当の徒歩道を含む国土地理院のレベル 16 から全国の道を補完する。"""
import argparse
from concurrent.futures import ThreadPoolExecutor,ProcessPoolExecutor
from contextlib import closing
import gzip
import json
from pathlib import Path
import shutil
import sqlite3
import time
import numpy as np
import shapely
from shapely.geometry import shape,box
from national_lines import source_index,batches,write_progress,WORLD,LEVELS,network_lines,prune_network,encode_lines,decode_lines,line_parts

TRAIL_CODES={2721,2722,2723,2724,2731,2732,2733,2734}


def strip_trails(raw):
    from mapbox_vector_tile.decoder import TileData
    decoder=TileData(raw,default_options={'y_coord_down':True})
    for i in reversed(range(len(decoder.tile.layers))):
        if decoder.tile.layers[i].name!='RdCL':del decoder.tile.layers[i]
    count=0
    for layer in decoder.tile.layers:
        if 'vt_code' not in layer.keys:del layer.features[:];continue
        key=list(layer.keys).index('vt_code')
        for i in reversed(range(len(layer.features))):
            tags=layer.features[i].tags;code=None
            for k,v in zip(tags[::2],tags[1::2]):
                if k==key:
                    value=layer.values[v]
                    for field in ['int_value','uint_value','sint_value','double_value']:
                        if value.HasField(field):code=int(getattr(value,field));break
                    break
            if code not in TRAIL_CODES:del layer.features[i]
        count+=len(layer.features)
    return gzip.compress(decoder.tile.SerializeToString(),mtime=0) if count else None


def extract(task):
    offset,length,tiles,cache=task;data=(Path(cache)/f'{offset}-{length}.range').read_bytes();result=[];seen={}
    for x,y,start,size in tiles:
        key=start,size
        if key not in seen:
            body=data[start-offset:start-offset+size]
            seen[key]=strip_trails(gzip.decompress(body) if body[:2]==b'\x1f\x8b' else body)
        body=seen[key]
        if body:result.append((x,y,body))
    return offset,len(tiles),result


def acquire(root,workers=4):
    from coastline_source import remote_reader
    root.mkdir(parents=True,exist_ok=True);cache=root/'source-ranges';reader,snapshot=remote_reader(cache)
    print('全国レベル 16 の索引を読み込みます',flush=True)
    tiles=source_index(reader,snapshot,root/'source-index16.json',16);groups=batches(tiles)
    print(json.dumps({'tiles':len(tiles),'chunks':len(groups),'bytes':sum(n for o,n,rows in groups)}),flush=True)
    started=time.monotonic()
    with closing(sqlite3.connect(root/'source-trails.sqlite3',timeout=60)) as db:
        db.execute('CREATE TABLE IF NOT EXISTS tiles(x INTEGER,y INTEGER,body BLOB,PRIMARY KEY(x,y)) WITHOUT ROWID')
        db.execute('CREATE TABLE IF NOT EXISTS chunks(offset INTEGER PRIMARY KEY,scanned INTEGER)')
        db.execute('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT)')
        stored=db.execute("SELECT value FROM metadata WHERE key='sourceSnapshot'").fetchone()
        if stored and json.loads(stored[0])!=snapshot:raise ValueError('徒歩道元データの更新時点が異なります')
        db.execute('INSERT OR REPLACE INTO metadata VALUES (?,?)',('sourceSnapshot',json.dumps(snapshot)));db.commit()
        saved=set(o for o, in db.execute('SELECT offset FROM chunks'));pending=[g for g in groups if g[0] not in saved]
        def fetch(task):
            o,n,rows=task
            if shutil.disk_usage(root).free<4*1024**3:raise ValueError('空き容量が 4 GB 未満になったため中断しました')
            getattr(reader.get_bytes,'__wrapped__',reader.get_bytes)(o,n);return n
        amount=0
        with ThreadPoolExecutor(max_workers=workers) as pool:
            for i,n in enumerate(pool.map(fetch,pending)):
                amount+=n
                if i%16==0 or i+1==len(pending):write_progress(root,'download-trails',i+1,len(pending),started,bytes=amount,sourceTiles=len(tiles))
        tasks=[(o,n,rows,str(cache)) for o,n,rows in pending];done=db.execute('SELECT coalesce(sum(scanned),0) FROM chunks').fetchone()[0]
        with ProcessPoolExecutor(max_workers=workers) as pool:
            for offset,scanned,rows in pool.map(extract,tasks):
                db.executemany('INSERT OR REPLACE INTO tiles VALUES (?,?,?)',rows);db.execute('INSERT INTO chunks VALUES (?,?)',(offset,scanned));db.commit();done+=scanned
                write_progress(root,'extract-trails',done,len(tiles),started,savedTiles=db.execute('SELECT count(*) FROM tiles').fetchone()[0])
        assert done==len(tiles)
        values={'complete':True,'sourceZoom':16,'sourceTileCount':len(tiles),'savedTileCount':db.execute('SELECT count(*) FROM tiles').fetchone()[0],'sourceSnapshot':snapshot}
        db.executemany('INSERT OR REPLACE INTO metadata VALUES (?,?)',[(k,json.dumps(v)) for k,v in values.items()]);db.commit()
        assert db.execute('PRAGMA quick_check').fetchone()[0]=='ok'
    return root/'source-trails.sqlite3'


def build_unit(task):
    from mapbox_vector_tile.decoder import TileData
    source,x,y=task[:3];selected_levels=set(task[3]) if len(task)>3 else set(LEVELS);size=WORLD/2**16;unit=WORLD/256
    boundary=box(x*unit-WORLD/2,WORLD/2-(y+1)*unit,(x+1)*unit-WORLD/2,WORLD/2-y*unit)
    with closing(sqlite3.connect(Path(source).resolve().as_uri()+'?mode=ro',uri=True)) as db:
        rows=db.execute('SELECT x,y,body FROM tiles WHERE x>=? AND x<? AND y>=? AND y<?',(x*256,(x+1)*256,y*256,(y+1)*256)).fetchall()
    geometries=[]
    for tx,ty,body in rows:
        layer=TileData(gzip.decompress(body),default_options={'y_coord_down':True}).get_message().get('RdCL',{})
        extent=layer.get('extent',4096)
        for item in layer.get('features',[]):
            geometry=shapely.transform(shape(item['geometry']),lambda p:np.column_stack([(tx+p[:,0]/extent)*size-WORLD/2,WORLD/2-(ty+p[:,1]/extent)*size])).intersection(boundary)
            geometries.extend(line for line in line_parts(geometry) if line.length)
    result=[]
    if geometries:
        network=network_lines(geometries)
        for level,(tolerance,minimum) in reversed(list(LEVELS.items())):
            network=prune_network(network,minimum,boundary)
            if level not in selected_levels:continue
            selected=[line.simplify(tolerance,preserve_topology=True) if tolerance else line for line in network]
            body,count,vertices=encode_lines(selected)
            if count:result.append((level,body,count,vertices))
    return x,y,result


def generate(root,base,output,workers=4):
    source=root/'source-trails.sqlite3';started=time.monotonic();temp=output.with_suffix('.part.sqlite3')
    with closing(sqlite3.connect(source.resolve().as_uri()+'?mode=ro',uri=True)) as src:
        meta={k:json.loads(v) for k,v in src.execute('SELECT key,value FROM metadata')};assert meta['complete']
        units=src.execute('SELECT DISTINCT x/256,y/256 FROM tiles ORDER BY x/256,y/256').fetchall()
    if not temp.exists():
        with closing(sqlite3.connect(base.resolve().as_uri()+'?mode=ro',uri=True)) as original,closing(sqlite3.connect(temp)) as db:
            base_meta={k:json.loads(v) for k,v in original.execute('SELECT key,value FROM metadata')}
            if base_meta['sourceSnapshot']!=meta['sourceSnapshot']:raise ValueError('河川・県境と徒歩道の更新時点が異なります')
            original.backup(db);db.execute("DELETE FROM line_bounds WHERE id IN (SELECT id FROM line_tiles WHERE kind='trail')");db.execute("DELETE FROM line_tiles WHERE kind='trail'");db.execute('CREATE TABLE trail_completed(x INTEGER,y INTEGER,PRIMARY KEY(x,y)) WITHOUT ROWID');db.execute('INSERT OR REPLACE INTO metadata VALUES (?,?)',('trailSourceSnapshot',json.dumps(meta['sourceSnapshot'])));db.commit()
    with closing(sqlite3.connect(temp,timeout=60)) as db:
        db.execute('PRAGMA journal_mode=WAL')
        assert json.loads(db.execute("SELECT value FROM metadata WHERE key='trailSourceSnapshot'").fetchone()[0])==meta['sourceSnapshot']
        saved=set(db.execute('SELECT x,y FROM trail_completed'));tasks=[(str(source),x,y) for x,y in units if (x,y) not in saved];done=len(saved)
        with ProcessPoolExecutor(max_workers=workers) as pool:
            for x,y,rows in pool.map(build_unit,tasks):
                for level,body,count,vertices in rows:
                    cur=db.execute("INSERT INTO line_tiles(level,kind,x,y,body,lines,vertices) VALUES (?,'trail',?,?,?,?,?)",(level,x,y,body,count,vertices))
                    coords=np.concatenate(decode_lines(body));west,east=np.degrees(coords[:,0].min()/6378137),np.degrees(coords[:,0].max()/6378137)
                    south,north=np.degrees(2*np.arctan(np.exp(np.array([coords[:,1].min(),coords[:,1].max()])/6378137))-np.pi/2)
                    db.execute('INSERT INTO line_bounds VALUES (?,?,?,?,?)',(cur.lastrowid,west,east,south,north))
                db.execute('INSERT INTO trail_completed VALUES (?,?)',(x,y));db.commit();done+=1;write_progress(root,'generate-trails',done,len(units),started)
        reports=[{'level':z,'kind':k,'tiles':n,'lines':lines,'vertices':vertices,'bodyBytes':size} for z,k,n,lines,vertices,size in db.execute('SELECT level,kind,count(*),sum(lines),sum(vertices),sum(length(body)) FROM line_tiles GROUP BY level,kind ORDER BY level,kind')]
        values={'layerReports':reports,'trailSourceZoom':16,'trailSourceTileCount':meta['sourceTileCount'],'trailSavedTileCount':meta['savedTileCount'],'trailCodes':sorted(TRAIL_CODES),'trailClassification':'RdCL 2721-2724,2731-2734。レベル17相当の vt_flag17 付き徒歩道もレベル16から取得。','trailComplete':True}
        db.executemany('INSERT OR REPLACE INTO metadata VALUES (?,?)',[(k,json.dumps(v)) for k,v in values.items()]);db.commit();assert db.execute('PRAGMA quick_check').fetchone()[0]=='ok'
        db.execute('PRAGMA wal_checkpoint(TRUNCATE)');db.execute('PRAGMA journal_mode=DELETE')
    temp.replace(output)
    with output.open('rb') as src,output.with_suffix('.sqlite3.gz').open('wb') as dst,gzip.GzipFile(fileobj=dst,mode='wb',mtime=0) as gz:shutil.copyfileobj(src,gz)
    report={'sqliteBytes':output.stat().st_size,'gzipBytes':output.with_suffix('.sqlite3.gz').stat().st_size,'levels':reports}
    output.with_suffix('.report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8');write_progress(root,'complete-trails',len(units),len(units),started,**report)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--root',type=Path,default=Path('build/terrain-national-lines'));parser.add_argument('--workers',type=int,default=4);parser.add_argument('--stage',choices=['all','acquire','generate'],default='all');parser.add_argument('--base',type=Path,default=Path('build/terrain-national-lines.sqlite3'));parser.add_argument('--output',type=Path,default=Path('build/terrain-national-lines-complete-trails.sqlite3'));args=parser.parse_args()
    if args.stage in ('all','acquire'):acquire(args.root,args.workers)
    if args.stage in ('all','generate'):generate(args.root,args.base,args.output,args.workers)
