"""既存の全国線データを保持し、保存済み元データから不足レベルだけを追加する。"""
import argparse
from concurrent.futures import ProcessPoolExecutor
from contextlib import closing
import gzip
import hashlib
import json
from pathlib import Path
import shutil
import sqlite3
import time

import numpy as np
from national_lines import LEVELS,decode_lines,build_unit,write_progress
from national_trails import build_unit as build_trails
from ridges import RADIUS


def digest_rows(db,excluded):
    h=hashlib.sha256()
    for z,k,x,y,body in db.execute('SELECT level,kind,x,y,body FROM line_tiles ORDER BY level,kind,x,y'):
        if z in excluded:continue
        h.update(json.dumps([z,k,x,y]).encode());h.update(body)
    return h.hexdigest()


def add(base,root,output,levels,workers=4):
    selected=set(levels)
    if not selected or not selected<=set(LEVELS):raise ValueError('追加するレベルが不正です')
    if output.resolve()==base.resolve():raise ValueError('入力と出力は別ファイルにしてください')
    started=time.monotonic();temp=output.with_suffix('.part.sqlite3')
    with closing(sqlite3.connect(base.resolve().as_uri()+'?mode=ro',uri=True)) as src:
        meta={k:json.loads(v) for k,v in src.execute('SELECT key,value FROM metadata')}
        if selected&set(meta['levels']):raise ValueError('追加するレベルが既にあります')
        original=digest_rows(src,set())
        if not temp.exists():
            with closing(sqlite3.connect(temp)) as dest:
                src.backup(dest)
                dest.execute('CREATE TABLE level_completed(stage TEXT,x INTEGER,y INTEGER,PRIMARY KEY(stage,x,y)) WITHOUT ROWID')
                dest.execute('INSERT OR REPLACE INTO metadata VALUES (?,?)',('levelAddition',json.dumps({'baseDigest':original,'levels':sorted(selected)})));dest.commit()
    with closing(sqlite3.connect(temp,timeout=60)) as db:
        db.execute('PRAGMA journal_mode=WAL')
        assert json.loads(db.execute("SELECT value FROM metadata WHERE key='levelAddition'").fetchone()[0])=={'baseDigest':original,'levels':sorted(selected)}
        for stage,source,divisor,convert in [('river-boundary',root/'source.sqlite3',64,build_unit),('trail',root/'source-trails.sqlite3',256,build_trails)]:
            with closing(sqlite3.connect(source.resolve().as_uri()+'?mode=ro',uri=True)) as src:
                smeta={k:json.loads(v) for k,v in src.execute('SELECT key,value FROM metadata')}
                assert smeta['complete'] and smeta['sourceSnapshot']==meta['sourceSnapshot']
                units=src.execute(f'SELECT DISTINCT x/{divisor},y/{divisor} FROM tiles ORDER BY x/{divisor},y/{divisor}').fetchall()
            saved=set(db.execute('SELECT x,y FROM level_completed WHERE stage=?',(stage,)))
            tasks=[(str(source),x,y,sorted(selected)) for x,y in units if (x,y) not in saved];done=len(saved)
            with ProcessPoolExecutor(max_workers=workers) as pool:
                for result in pool.map(convert,tasks):
                    x,y=result[:2]
                    if stage=='trail':rows=[(z,'trail',x,y,b,n,v) for z,b,n,v in result[2]]
                    else:rows=[row for row in result[2] if row[1]!='trail']
                    for z,kind,tx,ty,body,count,vertices in rows:
                        cursor=db.execute('INSERT INTO line_tiles(level,kind,x,y,body,lines,vertices) VALUES (?,?,?,?,?,?,?)',(z,kind,tx,ty,body,count,vertices))
                        coords=np.concatenate(decode_lines(body));west,east=np.degrees(coords[:,0].min()/RADIUS),np.degrees(coords[:,0].max()/RADIUS)
                        south,north=np.degrees(2*np.arctan(np.exp(np.array([coords[:,1].min(),coords[:,1].max()])/RADIUS))-np.pi/2)
                        db.execute('INSERT INTO line_bounds VALUES (?,?,?,?,?)',(cursor.lastrowid,west,east,south,north))
                    db.execute('INSERT INTO level_completed VALUES (?,?,?)',(stage,x,y));db.commit();done+=1
                    write_progress(root,'add-'+stage,done,len(units),started,levels=sorted(selected))
        assert original==digest_rows(db,selected)
        reports=[{'level':z,'kind':k,'tiles':n,'lines':lines,'vertices':vertices,'bodyBytes':size} for z,k,n,lines,vertices,size in db.execute('SELECT level,kind,count(*),sum(lines),sum(vertices),sum(length(body)) FROM line_tiles GROUP BY level,kind ORDER BY level,kind')]
        meta.update(levels=sorted(set(meta['levels'])|selected),layerReports=reports)
        meta['additionalLevelSettings']={str(z):LEVELS[z] for z in selected}
        db.executemany('INSERT OR REPLACE INTO metadata VALUES (?,?)',[(k,json.dumps(v,ensure_ascii=False)) for k,v in meta.items()]);db.commit()
        assert db.execute('PRAGMA quick_check').fetchone()[0]=='ok'
        db.execute('PRAGMA wal_checkpoint(TRUNCATE)');db.execute('PRAGMA journal_mode=DELETE')
    temp.replace(output)
    packed=output.with_suffix('.sqlite3.gz')
    with output.open('rb') as src,packed.open('wb') as dst,gzip.GzipFile(fileobj=dst,mode='wb',mtime=0) as gz:shutil.copyfileobj(src,gz)
    report={'sqliteBytes':output.stat().st_size,'gzipBytes':packed.stat().st_size,'levels':reports,'existingLevelsUnchanged':True,'baseDataDigest':original}
    output.with_suffix('.report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8');print(json.dumps(report),flush=True)
    return report


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--base',type=Path,default=Path('build/terrain-national-lines.sqlite3'));parser.add_argument('--root',type=Path,default=Path('build/terrain-national-lines'));parser.add_argument('--output',type=Path,default=Path('build/terrain-national-lines-with5-7.sqlite3'));parser.add_argument('--levels',type=int,nargs='+',default=[5,7],choices=list(LEVELS));parser.add_argument('--workers',type=int,default=4);args=parser.parse_args();add(args.base,args.root,args.output,args.levels,args.workers)
