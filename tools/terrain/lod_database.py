"""圧縮標高と縮尺別の概略標高を読み取り専用 SQLite にまとめる。"""
import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import gzip
import json
from pathlib import Path
import shutil
import sqlite3

import numpy as np
from compact_dem import decode, encode


def reduce_tile(values):
    blocks=values.reshape(128,2,128,2)
    valid=np.isfinite(blocks)
    counts=valid.sum(axis=(1,3))
    sums=np.where(valid,blocks,0).sum(axis=(1,3))
    return np.divide(sums,counts,out=np.full((128,128),np.nan),where=counts>0).astype(np.float32)


def parent_tile(children):
    result=np.full((256,256),np.nan,dtype=np.float32)
    for (dx,dy),body in children.items():
        result[dy*128:(dy+1)*128,dx*128:(dx+1)*128]=reduce_tile(decode(body))
    return result


def build(source, output, minimum=4, workers=8):
    output.parent.mkdir(parents=True,exist_ok=True)
    temporary=output.with_suffix('.part.sqlite3')
    if temporary.exists():
        raise ValueError('前回の未完成データがあります。別の出力先を指定してください')
    db=sqlite3.connect(temporary)
    db.execute('PRAGMA page_size=512')
    db.execute('PRAGMA journal_mode=DELETE')
    db.execute('CREATE TABLE tiles (id INTEGER PRIMARY KEY, z INTEGER NOT NULL, x INTEGER NOT NULL, y INTEGER NOT NULL, data BLOB NOT NULL)')
    db.execute('CREATE UNIQUE INDEX tile_coordinates ON tiles(z,x,y)')
    db.execute('CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL)')
    def record(key,value):db.execute('INSERT OR REPLACE INTO metadata VALUES (?,?)',(key,json.dumps(value,ensure_ascii=False)))
    record('schemaFormat','yamamuki-terrain');record('schemaVersion',1)
    record('status','building');record('heightQuantizationMeters',5)
    record('maximumZoom',14);record('minimumZoom',minimum)
    record('overviewReduction','2×2 の標高あり部分の平均。全点欠損なら欠損を維持')
    record('overviewAccuracy','概略表示用。ズーム14の最大2.5 mという丸め誤差上限は低縮尺には適用しない')
    record('source','国土地理院 地理院タイル DEM10B を加工して作成')
    files=sorted((source/'14').rglob('*.ymd'))
    for start in range(0,len(files),1024):
        db.executemany('INSERT INTO tiles(z,x,y,data) VALUES (?,?,?,?)',[(14,int(p.parent.name),int(p.stem),p.read_bytes()) for p in files[start:start+1024]])
        db.commit()
    levels=[]
    for z in range(14,minimum-1,-1):
        count,size=db.execute('SELECT count(*),sum(length(data)) FROM tiles WHERE z=?',(z,)).fetchone()
        levels.append({'zoom':z,'tiles':count,'payloadBytes':size})
        record('levels',levels);db.commit()
        print(json.dumps(levels[-1]),flush=True)
        if z==minimum:break
        parents=db.execute('SELECT DISTINCT x/2,y/2 FROM tiles WHERE z=? ORDER BY y/2,x/2',(z,)).fetchall()
        with ThreadPoolExecutor(max_workers=workers) as pool:
            for start in range(0,len(parents),128):
                batch=[]
                for x,y in parents[start:start+128]:
                    children={(cx-x*2,cy-y*2):body for cx,cy,body in db.execute('SELECT x,y,data FROM tiles WHERE z=? AND x BETWEEN ? AND ? AND y BETWEEN ? AND ?',(z,x*2,x*2+1,y*2,y*2+1))}
                    batch.append((x,y,children))
                encoded=pool.map(lambda row:(row[0],row[1],encode(parent_tile(row[2]),5,'gzip')),batch)
                db.executemany('INSERT INTO tiles(z,x,y,data) VALUES (?,?,?,?)',[(z-1,x,y,body) for x,y,body in encoded])
                db.commit()
    record('status','complete');record('createdAt',datetime.now(timezone.utc).isoformat());db.commit()
    assert db.execute('PRAGMA quick_check').fetchone()[0]=='ok'
    db.close();temporary.replace(output)
    with output.open('rb') as src,gzip.open(str(output)+'.gz','wb',compresslevel=6) as dest:shutil.copyfileobj(src,dest)
    report={'file':str(output),'bytes':output.stat().st_size,'gzipBytes':Path(str(output)+'.gz').stat().st_size,'levels':levels}
    output.with_suffix('.report.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
    print(json.dumps(report),flush=True)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,default=Path('build/terrain-compact'))
    parser.add_argument('--output',type=Path,default=Path('build/terrain-lod.sqlite3'))
    parser.add_argument('--minimum',type=int,default=4,choices=range(1,15))
    parser.add_argument('--workers',type=int,default=8,choices=range(1,9))
    args=parser.parse_args();build(args.source,args.output,args.minimum,args.workers)
