"""標高を量子化し、二次元の差分で小さく保存する。欠損は別に保存する。"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import gzip
import hashlib
import json
import lzma
from pathlib import Path
import struct
import zlib

import numpy as np
from ridges import decode_png

HEADER=struct.Struct('<4sHHfiBB')


def encode(heights, step=1., codec='gzip'):
    if not 0<step<=5:
        raise ValueError('標高の単位は 0 より大きく 5 m 以下にしてください')
    valid=np.isfinite(heights)
    quantized=np.rint(np.where(valid,heights,0)/step).astype(np.int32)
    base=int(quantized[valid].min()) if valid.any() else 0
    q=np.where(valid,quantized-base,0).astype(np.int32)
    # 左・上・左上から予測し、緩やかな斜面では差分を小さくする。
    delta=q.copy()
    delta[:,1:]-=q[:,:-1]
    delta[1:,:]-=q[:-1,:]
    delta[1:,1:]+=q[:-1,:-1]
    wide=int(delta.min()<-32768 or delta.max()>32767)
    missing=int(not valid.all())
    raw=delta.astype('<i4' if wide else '<i2').tobytes()
    if missing:raw+=np.packbits(~valid,bitorder='little').tobytes()
    if codec=='gzip':body=gzip.compress(raw,compresslevel=9,mtime=0);kind=0
    elif codec=='xz':body=lzma.compress(raw,preset=6);kind=1
    else:raise ValueError('未対応の圧縮形式です')
    return HEADER.pack(b'YMD1',*heights.shape,step,base,wide|(missing<<1),kind)+body


def decode(body):
    magic,rows,columns,step,base,flags,codec=HEADER.unpack_from(body)
    if magic!=b'YMD1' or not rows or not columns or rows*columns>4_000_000 or flags>3 or codec not in (0,1) or not 0<step<=5:
        raise ValueError('圧縮標高のヘッダーが不正です')
    raw=gzip.decompress(body[HEADER.size:]) if codec==0 else lzma.decompress(body[HEADER.size:])
    width=4 if flags&1 else 2
    length=rows*columns*width
    mask_length=(rows*columns+7)//8 if flags&2 else 0
    if len(raw)!=length+mask_length:
        raise ValueError('圧縮標高の長さが不正です')
    delta=np.frombuffer(raw[:length],dtype='<i4' if flags&1 else '<i2').reshape(rows,columns)
    values=(delta.cumsum(axis=0,dtype=np.int64).cumsum(axis=1)+base)*step
    values=values.astype(np.float32)
    if flags&2:
        missing=np.unpackbits(np.frombuffer(raw[length:],dtype=np.uint8),bitorder='little')[:rows*columns].reshape(rows,columns)
        values[missing.astype(bool)]=np.nan
    return values


def pack_tile(task):
    path,cache,output,step,codec=task
    original=decode_png(path.read_bytes())
    packed=encode(original,step,codec)
    restored=decode(packed)
    valid=np.isfinite(original)
    if not np.array_equal(valid,np.isfinite(restored)):
        raise ValueError(f'欠損位置が変わっています: {path}')
    error=np.abs(original[valid]-restored[valid])
    maximum=float(error.max()) if error.size else 0.
    if maximum>step/2+0.001:
        raise ValueError(f'許容誤差を超えています: {path}')
    relative=path.relative_to(cache).with_suffix('.ymd')
    dest=output/relative;dest.parent.mkdir(parents=True,exist_ok=True)
    temporary=dest.with_suffix('.part');temporary.write_bytes(packed);temporary.replace(dest)
    return {'path':relative.as_posix(),'bytes':len(packed),'sha256':hashlib.sha256(packed).hexdigest(),
            'maxErrorMeters':maximum,'sumSquaredError':float(np.square(error).sum()),'validSamples':int(valid.sum())}


def main(args):
    files=sorted(args.cache.rglob('*.png'))
    args.output.mkdir(parents=True,exist_ok=True)
    size=0;maximum=0.;squared=0.;samples=0;count=0
    with ThreadPoolExecutor(max_workers=args.workers) as pool,(args.output/'tiles.jsonl').open('w',encoding='utf-8',newline='\n') as inventory:
        for start in range(0,len(files),128):
            tasks=[(p,args.cache,args.output,args.step,args.codec) for p in files[start:start+128]]
            for row in pool.map(pack_tile,tasks):
                inventory.write(json.dumps(row)+'\n');size+=row['bytes'];count+=1
                maximum=max(maximum,row['maxErrorMeters']);squared+=row['sumSquaredError'];samples+=row['validSamples']
            if start%5120==0:print(count,'/',len(files),flush=True)
    report={'schemaVersion':1,'format':'YMD1','heightStepMeters':args.step,'codec':args.codec,'tileCount':count,
            'tileBytes':size,'maxErrorMeters':maximum,'rmseMeters':(squared/samples)**.5 if samples else 0,
            'missingSamplesPreserved':True,'horizontalDownsampling':False,
            'source':'国土地理院 地理院タイル DEM10B を加工して作成'}
    (args.output/'manifest.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8',newline='\n')
    print(json.dumps(report,ensure_ascii=False),flush=True)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cache',type=Path,default=Path('build/terrain-cache'))
    parser.add_argument('--output',type=Path,default=Path('build/terrain-compact'))
    parser.add_argument('--step',type=float,default=1.)
    parser.add_argument('--codec',choices=['gzip','xz'],default='gzip')
    parser.add_argument('--workers',type=int,choices=range(1,9),default=4)
    main(parser.parse_args())
