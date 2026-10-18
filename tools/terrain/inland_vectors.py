"""参照パックの RIV1/WAT1 から、比較範囲の河川線と内水面を用意する。"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import sqlite3
import struct
import zipfile

import numpy as np
import shapely
from shapely.geometry import Polygon, LineString, box
from sea_vectors import polygons

ROOT=Path(__file__).resolve().parents[2]
OUTPUT=ROOT/'build/terrain-inland-vectors'

class Reader:
    def __init__(self,data,magic):
        if data[:4]!=magic:raise ValueError('ベクター形式が違います')
        self.data=data;self.pos=4
    def uint(self):
        v=struct.unpack_from('<I',self.data,self.pos)[0];self.pos+=4;return v
    def points(self):
        n=self.uint()
        if n> (len(self.data)-self.pos)//4:raise ValueError('頂点数が不正です')
        p=np.frombuffer(self.data,dtype='<u2',count=n*2,offset=self.pos).reshape(-1,2).astype(np.float64)/4096
        self.pos+=n*4
        if np.any(p>1):raise ValueError('頂点がタイル範囲外です')
        return p
    def finish(self):
        if self.pos!=len(self.data):raise ValueError('ベクターの終端が不正です')

def decode_rivers(data):
    r=Reader(data,b'RIV1');out=[r.points() for _ in range(r.uint())];r.finish();return out

def decode_water(data):
    r=Reader(data,b'WAT1');out=[]
    for _ in range(r.uint()):
        kind=r.uint();rings=[r.points() for _ in range(r.uint())]
        if kind not in (0,1) or not rings:raise ValueError('水面の種別・リングが不正です')
        out.append((kind,rings))
    r.finish();return out

def import_source(source):
    OUTPUT.mkdir(parents=True,exist_ok=True)
    with source.open('rb') as f:digest=hashlib.file_digest(f,'sha256').hexdigest()
    side=source.with_suffix('.sha256')
    if side.exists() and side.read_text(encoding='utf-8-sig').strip().split()[0]!=digest:raise ValueError('参照パックのハッシュが一致しません')
    dest=OUTPUT/'source.sqlite3'
    with sqlite3.connect(dest) as db:
        db.execute('CREATE TABLE IF NOT EXISTS tiles(layer TEXT,z INTEGER,x INTEGER,y INTEGER,data BLOB,PRIMARY KEY(layer,z,x,y)) WITHOUT ROWID')
        db.execute('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT)')
        old=db.execute("SELECT value FROM metadata WHERE key='sha256'").fetchone()
        if old and old[0]!=digest:raise ValueError('別の入力は別の出力先へ保存してください')
        with zipfile.ZipFile(source) as archive:
            count=0
            for name in archive.namelist():
                if name.startswith(('rivers/','water/','sea/')) and name.endswith(('.riv','.wat')):
                    layer,z,x,y=name.replace('.riv','').replace('.wat','').split('/')
                    db.execute('INSERT OR IGNORE INTO tiles VALUES(?,?,?,?,?)',(layer,int(z),int(x),int(y),archive.read(name)));count+=1
                elif name in ('manifest.json','NOTICE.txt','ODbL-1.0.txt','rivers.geojson.gz','water.geojson.gz','source-lock.json','water-source-lock.json','water-skipped.json'):
                    (OUTPUT/('source-'+name)).write_bytes(archive.read(name))
            db.execute("INSERT OR REPLACE INTO metadata VALUES('sha256',?)",(digest,))
            db.execute("INSERT OR REPLACE INTO metadata VALUES('source',?)",(str(source),))
        db.commit()
    print('保存済みベクター:',count,'タイル')

def paths(geometry):
    if geometry.geom_type=='LineString':
        if not geometry.is_empty:yield geometry
    elif hasattr(geometry,'geoms'):
        for g in geometry.geoms:yield from paths(g)

def transform(points,z,x,y,bounds):
    west,north,spanx,spany=bounds
    result=(points+[x,y])/2**z
    return (result-[west,north])/[spanx,spany]

def make_packet(db,bounds,zoom):
    west,north,sx,sy=bounds;n=2**zoom
    lo_x,hi_x=math.floor(west*n),math.floor((west+sx)*n)
    lo_y,hi_y=math.floor(north*n),math.floor((north+sy)*n)
    layers=[[],[],[]];region=box(0,0,1,1);features=[0,0,0]
    rows=db.execute("SELECT layer,z,x,y,data FROM tiles WHERE layer IN ('rivers','water') AND z=? AND x BETWEEN ? AND ? AND y BETWEEN ? AND ?",(zoom,lo_x,hi_x,lo_y,hi_y))
    for layer,z,x,y,data in rows:
        if layer=='rivers':
            for points in decode_rivers(data):
                if len(points)<2:continue
                line=LineString(transform(points,z,x,y,bounds)).intersection(region)
                for part in paths(line):
                    # 地形の起伏に沿うよう、長い直線だけ細分化する。
                    p=np.asarray(part.segmentize(1/512).coords)
                    layers[0].extend(np.stack([p[:-1],p[1:]],axis=1).reshape(-1,2));features[0]+=1
        else:
            for kind,rings in decode_water(data):
                p=[transform(r,z,x,y,bounds) for r in rings]
                shape=shapely.make_valid(Polygon(p[0],p[1:])).intersection(region)
                for area in polygons(shape):
                    for t in shapely.constrained_delaunay_triangles(area).geoms:
                        layers[1+kind].extend(np.asarray(t.exterior.coords)[:3])
                    features[1+kind]+=1
    arrays=[np.asarray(p,dtype='<f4').reshape(-1,2) for p in layers]
    header=dict(version=1,zoom=zoom,counts=[len(p) for p in arrays],features=features,bounds=bounds,attribution='© OpenStreetMap contributors (ODbL)')
    encoded=json.dumps(header,separators=(',',':')).encode('utf-8')
    return struct.pack('<4sI',b'INL1',len(encoded))+encoded+b''.join(p.tobytes() for p in arrays),header

def generate():
    source_meta=json.loads((OUTPUT/'source-manifest.json').read_text(encoding='utf-8'))
    reports=[]
    with sqlite3.connect((OUTPUT/'source.sqlite3').as_uri()+'?mode=ro',uri=True) as db:
        jobs=[]
        national=json.loads((ROOT/'build/terrain-national-shading/manifest.json').read_text(encoding='utf-8'))
        jobs.append(('national',[national['west']/64,national['north']/64,national['width']/256/64,national['height']/256/64],10))
        for prefix,folder in [('native','terrain-tile-preview'),('region','terrain-heightmap')]:
            for area in ['yari','tsukuba']:
                for level in range(4,15):
                    path=ROOT/f'build/{folder}-{area}/levels/{level if prefix=="native" else 14}/manifest.json'
                    if not path.exists():continue
                    m=json.loads(path.read_text(encoding='utf-8'));n=2**m['zoom']
                    bounds=[m['west']/n,m['north']/n,m['width']/256/n,m['height']/256/n]
                    km=m['width']*m['meters']/1000
                    zoom=12 if km<=50 else 11 if km<=200 else 10
                    jobs.append((f'{prefix}-{area}-L{level}',bounds,zoom))
        cached={}
        for name,bounds,zoom in jobs:
            key=(*bounds,zoom)
            if key not in cached:cached[key]=make_packet(db,bounds,zoom)
            body,header=cached[key];(OUTPUT/f'{name}.inland').write_bytes(body)
            reports.append(dict(name=name,bytes=len(body),**header))
    with sqlite3.connect(OUTPUT/'source.sqlite3') as db:source_hash=db.execute("SELECT value FROM metadata WHERE key='sha256'").fetchone()[0]
    report=dict(sourceSHA256=source_hash,layers=reports,riverComplete=source_meta.get('rivers',{}).get('complete',False),source='参照先の保存済み RIV1/WAT1。追加のネットワーク取得なし。',attribution='© OpenStreetMap contributors (ODbL)')
    (OUTPUT/'manifest.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
    print('河川・内水面:',len(reports),'範囲')

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--source',type=Path);parser.add_argument('--import-only',action='store_true')
    args=parser.parse_args()
    if args.source:import_source(args.source)
    if not args.import_only:generate()
