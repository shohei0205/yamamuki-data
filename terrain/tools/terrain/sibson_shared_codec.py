"""親の全 XY を暗黙に共有し、共通点 Z 補正と追加点だけを保存する。"""
import lzma
import struct
import time
import numpy as np
from mls_quant_codec import encode_quant, decode_quant
from sibson_delta_codec import checksum
from sibson_surface import SibsonSurface

HEADER=struct.Struct('<4sdIII')
UNIT=.125


def xy_keys(points):
    v=np.rint(np.asarray(points)[:,:2]*16).astype(np.int64)+512000
    if np.any(v<0) or np.any(v>=2**32):raise ValueError('XY が範囲外です')
    return (v[:,0]<<32)|v[:,1]


def canonical(points):
    p=np.asarray(points,dtype='<f4')
    v=np.rint(p[:,:2]*16).astype(np.int64)+1000000
    if np.any(v<0) or np.any(v>=2**22):raise ValueError('空間順の範囲外です')
    key=np.zeros(len(p),dtype=np.uint64)
    for bit in range(22):
        key|=((v[:,0]>>bit)&1).astype(np.uint64)<<(2*bit)
        key|=((v[:,1]>>bit)&1).astype(np.uint64)<<(2*bit+1)
    return p[np.argsort(key,kind='stable')].copy()


def encode_shared(child,parent,parent_error):
    if not np.isfinite(parent_error) or parent_error<=0:raise ValueError('親の許容誤差が不正です')
    for p in [child,parent]:
        p=np.asarray(p)
        if p.ndim!=2 or p.shape[1]!=3 or not np.isfinite(p).all():raise ValueError('不正な点列です')
        if not np.array_equal(np.rint(p/[.0625,.0625,UNIT])*[.0625,.0625,UNIT],p):
            raise ValueError('点列が指定の刻みではありません')
    if len(parent)<3 or len(child)<len(parent):raise ValueError('親子の点数が不正です')
    child=canonical(child);parent=canonical(parent)
    ck,pk=xy_keys(child),xy_keys(parent);order=np.argsort(ck);sorted_keys=ck[order]
    if len(np.unique(ck))!=len(child) or len(np.unique(pk))!=len(parent):raise ValueError('XY が重複しています')
    found=np.searchsorted(sorted_keys,pk)
    if np.any(found>=len(child)) or not np.array_equal(sorted_keys[np.minimum(found,len(child)-1)],pk):
        raise ValueError('子が親の XY を全て含んでいません')
    indices=order[found]
    correction=np.rint((child[indices,2]-parent[:,2])/UNIT).astype('<i4')
    if not np.array_equal((parent[:,2]+correction*UNIT).astype('<f4'),child[indices,2]):
        raise ValueError('Z 補正が 0.125 m 刻みではありません')
    difference=np.diff(correction,prepend=np.int32(0)).astype('<i4')
    packed=lzma.compress(difference.view('u1').reshape(-1,4).T.copy().tobytes(),preset=6)
    mask=np.ones(len(child),bool);mask[indices]=False
    added=encode_quant(child[mask],xy=.0625,zstep=UNIT)
    return HEADER.pack(b'NSP1',parent_error,len(parent),int(mask.sum()),len(packed))+packed+added


def decode_shared(blob,parent,parent_error):
    if len(blob)<=HEADER.size:raise ValueError('共有点ヘッダーが不足しています')
    magic,pe,count,added_count,size=HEADER.unpack_from(blob)
    if magic!=b'NSP1' or pe!=parent_error or count!=len(parent):raise ValueError('親が一致しません')
    raw=lzma.decompress(blob[HEADER.size:HEADER.size+size])
    if len(raw)!=count*4:raise ValueError('Z 補正の点数が一致しません')
    difference=np.frombuffer(raw,'u1').reshape(4,count).T.copy().view('<i4').reshape(count)
    correction=np.cumsum(difference.astype(np.int64))
    inherited=canonical(parent);inherited[:,2]=(inherited[:,2]+correction*UNIT).astype('<f4')
    added=decode_quant(blob[HEADER.size+size:])
    if len(added)!=added_count:raise ValueError('追加点数が一致しません')
    result=canonical(np.vstack((inherited,added)))
    if len(np.unique(xy_keys(result)))!=len(result):raise ValueError('復元 XY が重複しています')
    return result


def load_shared(db,region,error):
    stats=dict(bytes=0,levels=[],decodeSeconds=0.,modelBuildSeconds=0.)
    started=time.perf_counter()
    def load(e,stack):
        if e in stack or len(stack)>=16:raise ValueError('親参照が循環または深すぎます')
        row=db.execute('SELECT parent_error,blob,sha256 FROM levels WHERE region=? AND error=?',(region,e)).fetchone()
        if row is None:raise ValueError('親レベルがありません')
        pe,blob,digest=row;stats['bytes']+=len(blob);stats['levels'].append(e)
        if pe is None:
            t=time.perf_counter();points=decode_quant(blob)
        else:
            if pe<=e:raise ValueError('親は粗いレベルにしてください')
            parent=load(pe,stack+[e]);t=time.perf_counter();points=decode_shared(blob,parent,pe)
        stats['decodeSeconds']+=time.perf_counter()-t
        if checksum(points)!=digest:raise ValueError('復元点の検査値が一致しません')
        return points
    points=load(error,[])
    t=time.perf_counter();model=SibsonSurface(points);stats['modelBuildSeconds']=time.perf_counter()-t
    stats['seconds']=time.perf_counter()-started
    return points,model,stats
