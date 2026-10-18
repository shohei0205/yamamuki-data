"""追加レベルの全データを検証し、既存レベルの本体が変わらないことを確認する。"""
import argparse
from contextlib import closing
import gzip
import hashlib
import json
from pathlib import Path
import sqlite3
import numpy as np
from national_lines import decode_lines
from polygon_dem import decode_mesh


def stream_digest(stream):
    result=hashlib.sha256()
    for block in iter(lambda:stream.read(1024*1024),b''):result.update(block)
    return result.hexdigest()


def verify(base,output,levels):
    levels=set(levels);counts={};vertices=0;added_rows=0
    with closing(sqlite3.connect(base.resolve().as_uri()+'?mode=ro',uri=True)) as old,closing(sqlite3.connect(output.resolve().as_uri()+'?mode=ro',uri=True)) as new:
        meta={k:json.loads(v) for k,v in new.execute('SELECT key,value FROM metadata')}
        terrain=meta['schemaFormat']=='yamamuki-terrain-mesh'
        assert new.execute('PRAGMA quick_check').fetchone()[0]=='ok'
        table='tiles' if terrain else 'line_tiles';column='z' if terrain else 'level'
        query='SELECT z,x,y,data FROM tiles ORDER BY z,x,y' if terrain else 'SELECT level,kind,x,y,body FROM line_tiles ORDER BY level,kind,x,y'
        def digest(db):
            h=hashlib.sha256();count=0
            for row in db.execute(query):
                if row[0] in levels:continue
                h.update(json.dumps(row[:-1]).encode());h.update(row[-1]);count+=1
            return count,h.hexdigest()
        old_count,old_digest=digest(old);new_count,new_digest=digest(new)
        assert old_count==new_count and old_digest==new_digest
        old_levels=set(z for z, in old.execute(f'SELECT DISTINCT {column} FROM {table}'))
        assert set(meta['levels'])==old_levels|levels
        assert set(z for z, in new.execute(f'SELECT DISTINCT {column} FROM {table}'))==old_levels|levels
        if terrain:
            for z,x,y,body in new.execute('SELECT z,x,y,data FROM tiles WHERE z IN ('+','.join('?'*len(levels))+')',tuple(levels)):
                points,faces=decode_mesh(body);assert np.isfinite(points).all();assert len(points)>0 and len(faces)>0
                assert faces.min()>=0 and faces.max()<len(points)
                xy=points[faces,:2];area=(xy[:,1,0]-xy[:,0,0])*(xy[:,2,1]-xy[:,0,1])-(xy[:,1,1]-xy[:,0,1])*(xy[:,2,0]-xy[:,0,0])
                assert np.all(np.abs(area)>1e-10)
                counts[str(z)]=counts.get(str(z),0)+1;vertices+=len(points);added_rows+=1
        else:
            for z,kind,body,n,v,rid in new.execute('SELECT level,kind,body,lines,vertices,id FROM line_tiles WHERE level IN ('+','.join('?'*len(levels))+')',tuple(levels)):
                lines=decode_lines(body);assert len(lines)==n and sum(len(line) for line in lines)==v
                assert new.execute('SELECT id FROM line_bounds WHERE id=?',(rid,)).fetchone()
                key=f'{z}/{kind}';counts[key]=counts.get(key,0)+1;vertices+=v;added_rows+=1
            for z in levels:
                for kind in ['river','trail','boundary']:assert counts.get(f'{z}/{kind}',0)>0
    with output.open('rb') as stream:original=stream_digest(stream)
    packed=output.with_suffix(output.suffix+'.gz')
    with gzip.open(packed,'rb') as stream:assert original==stream_digest(stream)
    with packed.open('rb') as stream:compressed=stream_digest(stream)
    result={'levelsAdded':sorted(levels),'addedRows':added_rows,'verticesDecoded':vertices,'addedCounts':counts,'existingRows':old_count,'existingLevelsUnchanged':True,'baseDataDigest':old_digest,'sqliteBytes':output.stat().st_size,'gzipBytes':packed.stat().st_size,'sqliteSha256':original,'gzipSha256':compressed,'quickCheck':'ok','gzipRoundtripVerified':True}
    output.with_suffix('.addition-verification.json').write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf-8');print(json.dumps(result),flush=True)
    return result


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--base',type=Path,required=True);parser.add_argument('--output',type=Path,required=True);parser.add_argument('--levels',type=int,nargs='+',default=[5,7]);args=parser.parse_args();verify(args.base,args.output,args.levels)
