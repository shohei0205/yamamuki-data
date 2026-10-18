"""生成済みの地形レベルを、既存データを保ったまま別ファイルへ統合する。"""
import argparse
from contextlib import closing
import gzip
import hashlib
import json
from pathlib import Path
import shutil
import sqlite3


def merge(base,addition,output):
    if output.exists() or output.resolve() in (base.resolve(),addition.resolve()):
        raise ValueError('出力先は存在しない別ファイルを指定してください')
    with closing(sqlite3.connect(addition.resolve().as_uri()+'?mode=ro',uri=True)) as src:
        extra={k:json.loads(v) for k,v in src.execute('SELECT key,value FROM metadata')}
        extra_levels={z for z, in src.execute('SELECT DISTINCT z FROM tiles')}
        if extra_levels!=set(extra['levels']):raise ValueError('追加データのレベルが一致しません')
        assert src.execute('PRAGMA quick_check').fetchone()[0]=='ok'
    shutil.copyfile(base,output)
    with closing(sqlite3.connect(output)) as db:
        meta={k:json.loads(v) for k,v in db.execute('SELECT key,value FROM metadata')}
        for key in ('schemaFormat','schemaVersion','encoding'):
            if meta[key]!=extra[key]:raise ValueError('地形データの形式が一致しません')
        if set(meta['levels'])&extra_levels:raise ValueError('同じレベルが既にあります')
        db.execute('ATTACH DATABASE ? AS additional',(str(addition.resolve()),))
        db.execute('INSERT INTO tiles(z,x,y,data) SELECT z,x,y,data FROM additional.tiles')
        meta['levels']=sorted(set(meta['levels'])|extra_levels,reverse=True)
        meta['meshErrorBounds'].update(extra['meshErrorBounds'])
        meta.setdefault('additionalLevelSources',{}).update({str(z):extra.get('sourceFingerprint') for z in extra_levels})
        if extra.get('coastlineToleranceMeters'):
            if not extra.get('coastlineSource',{}).get('sourceSnapshot') or meta.get('coastlineSource',{}).get('sourceSnapshot')!=extra.get('coastlineSource',{}).get('sourceSnapshot'):raise ValueError('海岸線の取得時点が一致しません')
            meta.setdefault('coastlineToleranceMeters',{}).update(extra['coastlineToleranceMeters'])
            meta['coastlineSource']=extra['coastlineSource']
        db.executemany('INSERT OR REPLACE INTO metadata VALUES (?,?)',[(k,json.dumps(v,ensure_ascii=False)) for k,v in meta.items()]);db.commit()
        assert db.execute('PRAGMA quick_check').fetchone()[0]=='ok'
        rows=db.execute('SELECT z,count(*),sum(length(data)) FROM tiles GROUP BY z ORDER BY z DESC').fetchall()
    compressed=Path(str(output)+'.gz')
    with output.open('rb') as src,gzip.open(compressed,'wb',compresslevel=6) as dest:shutil.copyfileobj(src,dest)
    def digest(stream):
        h=hashlib.sha256()
        for block in iter(lambda:stream.read(1024*1024),b''):h.update(block)
        return h.hexdigest()
    with output.open('rb') as src:original=digest(src)
    with gzip.open(compressed,'rb') as src:assert digest(src)==original
    with compressed.open('rb') as src:packed=digest(src)
    report={'levels':rows,'sqliteBytes':output.stat().st_size,'gzipBytes':compressed.stat().st_size,'sqliteSha256':original,'gzipSha256':packed,'quickCheck':'ok','gzipRoundtripVerified':True}
    output.with_suffix('.verification.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
    print(json.dumps(report),flush=True)
    return report

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base',type=Path,required=True);parser.add_argument('--addition',type=Path,required=True);parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();merge(args.base,args.addition,args.output)
