"""地域比較用に、保存済み標高を可逆 PNG と法線 PNG へ変換する。"""
import argparse
from contextlib import closing
import json
import math
from pathlib import Path
import sqlite3

import numpy as np
from PIL import Image
from compact_dem import decode


def export(source, output):
    output.mkdir(parents=True,exist_ok=True)
    with closing(sqlite3.connect(source.resolve().as_uri()+'?mode=ro',uri=True)) as db:
        rows=db.execute('SELECT x,y,data FROM tiles WHERE z=14 ORDER BY y,x').fetchall()
    west,north=min(r[0] for r in rows),min(r[1] for r in rows)
    east,south=max(r[0] for r in rows),max(r[1] for r in rows)
    heights=np.full(((south-north+1)*256,(east-west+1)*256),np.nan,np.float32)
    for x,y,body in rows:
        heights[(y-north)*256:(y-north+1)*256,(x-west)*256:(x-west+1)*256]=decode(body)
    valid=np.isfinite(heights)
    # 比較元は 5 m 単位だが、PNG 自体は 1 m 単位で保持する。
    offset=math.floor(float(heights[valid].min()))
    q=np.rint(np.where(valid,heights-offset,0)).astype(np.int64)
    if q.max()>65535:raise ValueError('標高の範囲が 16 bit を超えています')
    pixels=np.stack([q&255,q>>8,valid.astype(np.uint8)*255],axis=-1).astype(np.uint8)
    Image.fromarray(pixels).save(output/'height.png',optimize=True)
    restored=pixels[:,:,0].astype(np.int64)+pixels[:,:,1].astype(np.int64)*256+offset
    error=float(np.abs(restored[valid]-heights[valid]).max())
    # 標高の上下に欠損がある場合は片側差分とし、欠損から架空の崖を作らない。
    center_lat=math.atan(math.sinh(math.pi*(1-2*(north+south+1)/2/2**14)))
    meters=2*math.pi*6378137*math.cos(center_lat)/(256*2**14)
    padded=np.pad(heights,1,mode='edge')
    a,b=padded[1:-1,:-2],padded[1:-1,2:]
    c,d=padded[:-2,1:-1],padded[2:,1:-1]
    a=np.where(np.isfinite(a),a,heights);b=np.where(np.isfinite(b),b,heights)
    c=np.where(np.isfinite(c),c,heights);d=np.where(np.isfinite(d),d,heights)
    dx=np.where(valid,(b-a)/(2*meters),0);dy=np.where(valid,(d-c)/(2*meters),0)
    length=np.sqrt(1+dx*dx+dy*dy)
    # 細部の向きを求めてから 2×2 点を平均し、法線 PNG は標高の半分の幅にする。
    h,w=heights.shape
    nx=(-dx/length).reshape(h//2,2,w//2,2).mean(axis=(1,3))
    ny=(dy/length).reshape(h//2,2,w//2,2).mean(axis=(1,3))
    normals=np.zeros((h//2,w//2,3),np.uint8)
    normals[:,:,:2]=np.rint((np.stack([nx,ny],axis=-1)+1)*127.5).clip(0,255).astype(np.uint8)
    normals[:,:,2]=255
    Image.fromarray(normals).save(output/'normal.png',optimize=True)
    coarse=np.empty((h//4,w//4,3),np.uint8)
    coarse[:,:,:2]=np.rint(normals[:,:,:2].astype(np.float32).reshape(h//4,2,w//4,2,2).mean(axis=(1,3))).astype(np.uint8)
    coarse[:,:,2]=255
    Image.fromarray(coarse).save(output/'normal-coarse.png',optimize=True)
    # PNG の保存・読み出しでも符号化した全画素が変わらないことを確認する。
    for name,expected in [('height.png',pixels),('normal.png',normals)]:
        with Image.open(output/name) as image:
            if not np.array_equal(np.asarray(image),expected):raise ValueError('PNG の画素が変わりました')
    meta=dict(zoom=14,west=west,north=north,width=w,height=h,heightOffset=offset,heightScale=1,
              heightEncoding='R + 256*G + offset; B=255 valid',normalEncoding='RG normal east,north; reconstruct positive Z',
              normalWidth=w//2,normalHeight=h//2,meters=meters,sourceHeightBytes=sum(len(r[2]) for r in rows),
              heightBytes=(output/'height.png').stat().st_size,normalBytes=(output/'normal.png').stat().st_size,
              coarseNormalBytes=(output/'normal-coarse.png').stat().st_size,
              pngMaxHeightErrorMeters=error,source='国土地理院 DEM10B・既存の量子化済み標高')
    (output/'manifest.json').write_text(json.dumps(meta,ensure_ascii=False,indent=2),encoding='utf-8')
    print(json.dumps(meta,ensure_ascii=False))


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--source',type=Path,default=Path('build/terrain-yari-preview-source.sqlite3'))
    p.add_argument('--output',type=Path,default=Path('build/terrain-heightmap-yari'))
    args=p.parse_args();export(args.source,args.output)
