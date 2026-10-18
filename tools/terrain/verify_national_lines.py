"""全国線データの全レベル、圧縮、索引、端点と取得件数を検証する。"""
import argparse
from contextlib import closing
import gzip
import hashlib
import json
import math
from pathlib import Path
import sqlite3

import numpy as np
from national_lines import LEVELS,KINDS,WORLD,decode_lines


def sha(path):
    h=hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda:f.read(1024*1024),b''):h.update(block)
    return h.hexdigest()


def verify(path,source):
    signatures={};rows_count=0;vertices_count=0;source_count=None
    with closing(sqlite3.connect(source.resolve().as_uri()+'?mode=ro',uri=True)) as db:
        source_count=db.execute('SELECT count(*) FROM tiles').fetchone()[0]
        meta_source={k:json.loads(v) for k,v in db.execute('SELECT key,value FROM metadata')}
        unit_count=db.execute('SELECT count(*) FROM (SELECT DISTINCT x/64,y/64 FROM tiles)').fetchone()[0]
        assert meta_source['complete'] and source_count==meta_source['sourceTileCount']
    with closing(sqlite3.connect(path.resolve().as_uri()+'?mode=ro',uri=True)) as db:
        assert db.execute('PRAGMA quick_check').fetchone()[0]=='ok'
        meta={k:json.loads(v) for k,v in db.execute('SELECT key,value FROM metadata')}
        assert meta['schemaFormat']=='yamamuki-national-lines' and meta['complete']
        assert meta['sourceTileCount']==source_count and meta['sourceSnapshot']==meta_source['sourceSnapshot']
        assert db.execute('SELECT count(*) FROM completed').fetchone()[0]==unit_count
        if meta.get('trailComplete'):
            with closing(sqlite3.connect((source.parent/'source-trails.sqlite3').resolve().as_uri()+'?mode=ro',uri=True)) as trails:
                trail_meta={k:json.loads(v) for k,v in trails.execute('SELECT key,value FROM metadata')}
                assert trail_meta['complete'] and trail_meta['sourceSnapshot']==meta['trailSourceSnapshot']==meta['sourceSnapshot']
                assert trails.execute('SELECT sum(scanned) FROM chunks').fetchone()[0]==meta['trailSourceTileCount']
                assert trails.execute('SELECT count(*) FROM tiles').fetchone()[0]==meta['trailSavedTileCount']
                trail_units=trails.execute('SELECT count(*) FROM (SELECT DISTINCT x/256,y/256 FROM tiles)').fetchone()[0]
                assert db.execute('SELECT count(*) FROM trail_completed').fetchone()[0]==trail_units
        for kind in KINDS:assert [z for z, in db.execute('SELECT DISTINCT level FROM line_tiles WHERE kind=? ORDER BY level',(kind,))]==list(LEVELS)
        for rid,z,kind,x,y,body,count,vertices in db.execute('SELECT * FROM line_tiles'):
            lines=decode_lines(body);assert len(lines)==count;assert sum(len(g) for g in lines)==vertices
            q=WORLD/2**meta['unitZoom'];left=round(x*q-WORLD/2);right=round((x+1)*q-WORLD/2);top=round(WORLD/2-y*q);bottom=round(WORLD/2-(y+1)*q)
            side={key:set() for key in ['west','east','south','north']}
            for line in lines:
                assert np.isfinite(line).all()
                for point in [line[0],line[-1]]:
                    px,py=map(int,point)
                    if px==left:side['west'].add((px,py))
                    if px==right:side['east'].add((px,py))
                    if py==bottom:side['south'].add((px,py))
                    if py==top:side['north'].add((px,py))
            signatures[(z,kind,x,y)]=side;rows_count+=1;vertices_count+=vertices
        assert rows_count==db.execute('SELECT count(*) FROM line_bounds').fetchone()[0]
        assert not db.execute('SELECT t.id FROM line_tiles t LEFT JOIN line_bounds b ON b.id=t.id WHERE b.id IS NULL LIMIT 1').fetchone()
    changes=[];seams=[]
    for (z,kind,x,y),sides in signatures.items():
        if z!=14:
            full=signatures.get((14,kind,x,y),{})
            for side,points in full.items():
                difference=points-sides.get(side,set())
                if difference:changes.append({'level':z,'kind':kind,'x':x,'y':y,'side':side,'missing':len(difference)})
        for dx,dy,a,b in [(1,0,'east','west'),(0,1,'south','north')]:
            other=signatures.get((z,kind,x+dx,y+dy))
            if other is not None:
                difference=sides[a]^other[b]
                if difference:seams.append({'level':z,'kind':kind,'x':x,'y':y,'side':a,'unmatched':len(difference)})
    digest=sha(path);h=hashlib.sha256()
    with gzip.open(path.with_suffix(path.suffix+'.gz'),'rb') as f:
        for block in iter(lambda:f.read(1024*1024),b''):h.update(block)
    assert digest==h.hexdigest()
    report={'sourceTiles':source_count,'trailSourceTiles':meta.get('trailSourceTileCount'),'trailSavedTiles':meta.get('trailSavedTileCount'),'sourceUnits':unit_count,'lineTiles':rows_count,'verticesAcrossLevels':vertices_count,'sqliteBytes':path.stat().st_size,'gzipBytes':path.with_suffix(path.suffix+'.gz').stat().st_size,'sqliteSha256':digest,'gzipSha256':sha(path.with_suffix(path.suffix+'.gz')),'quickCheck':'ok','gzipRoundtripVerified':True,'changedBoundaryEndpoints':changes,'unmatchedNeighborEndpoints':seams}
    path.with_suffix('.verification.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
    print(json.dumps({**report,'changedBoundaryEndpoints':len(changes),'unmatchedNeighborEndpoints':len(seams)},ensure_ascii=False))
    return report


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--database',type=Path,default=Path('build/terrain-national-lines.sqlite3'))
    parser.add_argument('--source',type=Path,default=Path('build/terrain-national-lines/source.sqlite3'))
    args=parser.parse_args();verify(args.database,args.source)
