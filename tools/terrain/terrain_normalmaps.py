"""詳細な標高の面の向きを、低レベル地形用の陰影マップへ保存する。"""
from contextlib import closing
import gzip
import json
import math
from pathlib import Path
import sqlite3

import numpy as np
from compact_dem import decode
from ridges import RADIUS


def normal_pixels(grid,meters,size):
    """周囲 1 点を含む標高から、東・北向きの単位法線を RG8 へ変換する。"""
    n=grid.shape[0]-2
    if grid.shape!=(n+2,n+2) or n<=0 or n%size:raise ValueError('陰影の標高配列と出力サイズが一致しません')
    spacing=np.asarray(meters,dtype=np.float32)
    if not np.isfinite(spacing).all() or np.any(spacing<=0):raise ValueError('標高の間隔が不正です')
    if spacing.ndim:spacing=spacing.reshape(n,1)
    dx=(grid[1:-1,2:]-grid[1:-1,:-2])/(2*spacing)
    dy=(grid[2:,1:-1]-grid[:-2,1:-1])/(2*spacing)
    valid=np.isfinite(grid[1:-1,1:-1])&np.isfinite(dx)&np.isfinite(dy)
    dx=np.where(valid,dx,0);dy=np.where(valid,dy,0)
    length=np.sqrt(1+dx*dx+dy*dy)
    factor=n//size
    # 海・欠損は上向きの法線とし、標高のない海岸に架空の崖を作らない。
    east=(-dx/length).reshape(size,factor,size,factor).mean(axis=(1,3))
    north=(dy/length).reshape(size,factor,size,factor).mean(axis=(1,3))
    return np.rint((np.stack([east,north],axis=-1)+1)*127.5).clip(0,255).astype(np.uint8)


def build(source,mesh,output,size=1024,source_zoom=8):
    factor=2**(source_zoom-4);n=factor*256
    if size not in (512,1024,2048) or n%size:raise ValueError('陰影の大きさが不正です')
    temporary=output.with_suffix('.part.sqlite3')
    if temporary.exists() or output.exists():raise ValueError('出力先が既にあります')
    with closing(sqlite3.connect(mesh.resolve().as_uri()+'?mode=ro',uri=True)) as terrain:
        tiles=terrain.execute('SELECT x,y FROM tiles WHERE z=4 ORDER BY x,y').fetchall()
    with closing(sqlite3.connect(source.resolve().as_uri()+'?mode=ro',uri=True)) as src,closing(sqlite3.connect(temporary)) as dest:
        dest.executescript('CREATE TABLE tiles(z INTEGER,x INTEGER,y INTEGER,width INTEGER,height INTEGER,data BLOB,PRIMARY KEY(z,x,y));CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT);')
        reports=[]
        for x,y in tiles:
            left=x*n-1;top=y*n-1;grid=np.full((n+2,n+2),np.nan,np.float32)
            for sy in range(top//256,(top+n+1)//256+1):
                for sx in range(left//256,(left+n+1)//256+1):
                    row=src.execute('SELECT data FROM tiles WHERE z=? AND x=? AND y=?',(source_zoom,sx,sy)).fetchone()
                    if not row:continue
                    tile=decode(row[0]);x0=max(left,sx*256);y0=max(top,sy*256);x1=min(left+n+2,(sx+1)*256);y1=min(top+n+2,(sy+1)*256)
                    grid[y0-top:y1-top,x0-left:x1-left]=tile[y0-sy*256:y1-sy*256,x0-sx*256:x1-sx*256]
            rows=y*n+np.arange(n)+.5
            latitude=np.arctan(np.sinh(math.pi*(1-2*rows/(256*2**source_zoom))))
            meters=2*math.pi*RADIUS*np.cos(latitude)/(256*2**source_zoom)
            pixels=normal_pixels(grid,meters,size);body=gzip.compress(pixels.tobytes(),compresslevel=6,mtime=0)
            dest.execute('INSERT INTO tiles VALUES (4,?,?,?,?,?)',(x,y,size,size,body));dest.commit()
            reports.append({'x':x,'y':y,'bytes':len(body)});print(json.dumps(reports[-1]),flush=True)
        stat=source.stat()
        meta={'schemaFormat':'yamamuki-terrain-normalmaps','schemaVersion':1,'level':4,'sourceZoom':source_zoom,'width':size,'height':size,'encoding':'gzip-rg8-normal-xy','sourceFingerprint':{'bytes':stat.st_size,'modifiedNs':stat.st_mtime_ns},'notes':'標高を変えず、面の向きだけを補う。東・北の単位法線を RG8 に保存。海・欠損は上向き。'}
        dest.executemany('INSERT INTO metadata VALUES (?,?)',[(k,json.dumps(v,ensure_ascii=False)) for k,v in meta.items()]);dest.commit()
        if dest.execute('PRAGMA quick_check').fetchone()[0]!='ok':raise ValueError('陰影マップの SQLite が不正です')
    temporary.replace(output)
    report={**meta,'tiles':reports,'bodyBytes':sum(row['bytes'] for row in reports),'sqliteBytes':output.stat().st_size}
    output.with_suffix('.report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
    print(json.dumps(report,ensure_ascii=False),flush=True)


if __name__=='__main__':
    import argparse
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,default=Path('build/terrain-lod.sqlite3'))
    parser.add_argument('--mesh',type=Path,default=Path('build/terrain-mesh.sqlite3'))
    parser.add_argument('--output',type=Path,default=Path('build/terrain-normalmaps.sqlite3'))
    parser.add_argument('--size',type=int,default=1024)
    args=parser.parse_args();build(args.source,args.mesh,args.output,args.size)
