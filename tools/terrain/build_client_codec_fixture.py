"""ブラウザーの復元結果を既存の Python 実装と比較する見本を作る。"""
import base64
import hashlib
import json
import math
from pathlib import Path
import sqlite3
from contextlib import closing

import numpy as np
from polygon_dem import decode_mesh
from polygon_region import payload
from ridges import tile_position
from national_lines import encode_lines,decode_lines
from shapely.geometry import LineString


def main():
    database=Path('build/terrain-mesh.sqlite3')
    result={'tiles':[],'regions':[]}
    line_body,_,_=encode_lines([LineString([(20000000,4000000),(20000001,4000002),(19999999,-2000000)]),LineString([(19999999,-2000000),(-1000000,3000000)])])
    decoded=np.concatenate(decode_lines(line_body)).astype('<f8')
    result['lines']={'body':base64.b64encode(line_body).decode(),'sha':hashlib.sha256(decoded.tobytes()).hexdigest()}
    with closing(sqlite3.connect(database.resolve().as_uri()+'?mode=ro',uri=True)) as db:
        result['metadata']={k:json.loads(v) for k,v in db.execute('SELECT key,value FROM metadata')}
        for z in range(4,15):
            x,y=map(int,tile_position(137.6476,36.32155,z))
            body=db.execute('SELECT data FROM tiles WHERE z=? AND x=? AND y=?',(z,x,y)).fetchone()[0]
            points,indices=decode_mesh(body)
            result['tiles'].append({'z':z,'x':x,'y':y,'body':base64.b64encode(body).decode(),
                'pointsSha':hashlib.sha256(points.astype('<f4').tobytes()).hexdigest(),
                'indicesSha':hashlib.sha256(indices.astype('<u4').tobytes()).hexdigest()})
        for args in [(36.32155,137.6476,30,12,None),(36.32155,137.6476,30,12,(80,180,12000)),(35.6,139.8,30,4,None)]:
            lat,lon,size,z,view=args
            reference=payload(database,lat,lon,size,z,view,outline=True)
            records=[];delta=math.degrees(size*500/6378137)
            for level in range(max(4,z-2),min(14,z+1)+1):
                a=tile_position(lon-delta/math.cos(math.radians(lat)),lat+delta,level)
                b=tile_position(lon+delta/math.cos(math.radians(lat)),lat-delta,level)
                for y in range(math.floor(a[1]),math.floor(b[1])+1):
                    for x in range(math.floor(a[0]),math.floor(b[0])+1):
                        row=db.execute('SELECT data FROM tiles WHERE z=? AND x=? AND y=?',(level,x,y)).fetchone()
                        if row:records.append({'z':level,'x':x,'y':y,'body':base64.b64encode(row[0]).decode()})
            stride=max(1,len(reference['mesh'])//30)
            result['regions'].append({'args':args,'records':records,'expected':{'coverage':reference['coverage'],
                'projectionCenter':reference['projectionCenter'],'vertices':len(reference['mesh']),
                'indicesSha':hashlib.sha256(np.asarray(reference['indices'],dtype='<u4').tobytes()).hexdigest(),
                'sample':reference['mesh'][::stride],'stride':stride}})
    output=Path('build/terrain-national/client-codec-fixture.json');output.parent.mkdir(parents=True,exist_ok=True)
    output.write_text(json.dumps(result,separators=(',',':')),encoding='utf-8')
    print(f'見本を保存しました: {output}（{output.stat().st_size} バイト）')


if __name__=='__main__':main()
