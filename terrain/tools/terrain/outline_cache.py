"""元データと一致する事前生成済み輪郭の差分を読み込む。"""
from contextlib import closing
from functools import lru_cache
import gzip
import json
import sqlite3
import numpy as np


def load(source):
    cache=source.with_name('terrain-outlines.sqlite3')
    if not cache.exists():return {},set()
    a=source.stat();b=cache.stat()
    return _load(str(cache.resolve()),a.st_size,a.st_mtime_ns,b.st_size,b.st_mtime_ns)


@lru_cache(maxsize=2)
def _load(path,source_bytes,source_modified,cache_bytes,cache_modified):
    with closing(sqlite3.connect('file:'+path.replace('\\','/')+'?mode=ro',uri=True)) as db:
        meta={k:json.loads(v) for k,v in db.execute('SELECT key,value FROM metadata')}
        if meta.get('schemaFormat')!='yamamuki-terrain-outline-offsets' or meta.get('schemaVersion')!=1:return {},set()
        if meta.get('sourceFingerprint')!={'bytes':source_bytes,'modifiedNs':source_modified}:return {},set()
        offsets={}
        for z,x,y,body in db.execute('SELECT z,x,y,data FROM tiles'):
            rows=np.frombuffer(gzip.decompress(body),dtype='<f4').reshape(-1,4)
            if not np.isfinite(rows).all():raise ValueError('輪郭の差分が不正です')
            offsets[z,x,y]=rows
        return offsets,set(meta['levels'])
