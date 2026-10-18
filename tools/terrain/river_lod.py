"""保存済み河川を接続した線へまとめ、合流点を保つ複数レベルを生成する。"""
import argparse
from contextlib import closing
import gzip
import json
import math
from pathlib import Path
import sqlite3

import numpy as np
import shapely
from shapely.geometry import shape, mapping, LineString
from ridges import RADIUS

# （線の許容誤差 m、短い末端の枝を省く長さ m）。河川の等級ではない。
LEVELS={4:(2000,10000),6:(800,5000),8:(250,2000),10:(60,500),12:(12,100),14:(0,0)}


def choose_level(size_km,latitude):
    desired=math.floor(math.log2(40075016.686*math.cos(math.radians(latitude))*1800/(max(2,size_km)*1000*256)))
    return max((z for z in LEVELS if z<=desired),default=4)


def line_parts(geometry):
    if geometry.geom_type=='LineString':yield geometry
    elif hasattr(geometry,'geoms'):
        for child in geometry.geoms:yield from line_parts(child)


def network_lines(geometries):
    # 元タイルの切れ目を結合し、交点で分割する。degree=2 の途中点だけを結合する。
    network=shapely.union_all([shapely.set_precision(g,.01) for g in geometries])
    return list(line_parts(shapely.line_merge(network)))


def simplify_network(lines,tolerance,minimum,coverage):
    degrees={}
    def ends(line):return tuple(line.coords[0]),tuple(line.coords[-1])
    for line in lines:
        for end in ends(line):degrees[end]=degrees.get(end,0)+1
    result=[]
    for line in lines:
        a,b=ends(line)
        # 取得範囲で切れた線を、短い支流と誤認して消さない。
        boundary=any(coverage.boundary.distance(shapely.Point(end))<.1 for end in (a,b))
        leaf=degrees[a]==1 or degrees[b]==1
        if minimum and leaf and line.length<minimum and not boundary:continue
        # 折れ線の両端（合流点・河口・取得範囲端）は Douglas-Peucker 法でも保持される。
        result.append(line.simplify(tolerance,preserve_topology=True) if tolerance else line)
    return result


def generate(source,output):
    if source.resolve()==output.resolve():raise ValueError('入力と出力は別ファイルにしてください')
    output.parent.mkdir(parents=True,exist_ok=True)
    temp=output.with_suffix('.part.sqlite3')
    if temp.exists():temp.unlink()
    with closing(sqlite3.connect(source.resolve().as_uri()+'?mode=ro',uri=True)) as src,closing(sqlite3.connect(temp)) as db:
        src.backup(db)
        meta={k:json.loads(v) for k,v in db.execute('SELECT key,value FROM metadata')}
        west,south,east,north=meta['bounds'];lon=(west+east)/2;lat=(south+north)/2
        sx=math.pi/180*RADIUS*math.cos(math.radians(lat));sy=math.pi/180*RADIUS
        def forward(x,y,z=None):return (np.asarray(x)-lon)*sx,(np.asarray(y)-lat)*sy
        def backward(x,y,z=None):return np.asarray(x)/sx+lon,np.asarray(y)/sy+lat
        geometries=[shapely.transform(shape(json.loads(gzip.decompress(body))['geometry']),forward,interleaved=False) for body, in db.execute("SELECT body FROM features WHERE kind='river'")]
        if not geometries:raise ValueError('河川の元データがありません')
        lines=network_lines(geometries)
        coverage=shapely.transform(shapely.box(west,south,east,north),forward,interleaved=False)
        db.execute('DROP TABLE IF EXISTS river_lod')
        db.execute('CREATE TABLE river_lod(level INTEGER,west REAL,south REAL,east REAL,north REAL,body BLOB)')
        db.execute('CREATE INDEX river_lod_level ON river_lod(level)')
        reports=[]
        for level,(tolerance,minimum) in LEVELS.items():
            selected=simplify_network(lines,tolerance,minimum,coverage)
            size=0;vertices=0
            for line in selected:
                geographic=shapely.transform(line,backward,interleaved=False)
                raw=json.dumps({'geometry':mapping(geographic),'properties':{'vt_code':5301}},separators=(',',':')).encode()
                body=gzip.compress(raw,mtime=0);size+=len(body);vertices+=len(line.coords)
                db.execute('INSERT INTO river_lod VALUES (?,?,?,?,?,?)',(level,*geographic.bounds,body))
            reports.append({'level':level,'lines':len(selected),'vertices':vertices,'bodyBytes':size,'toleranceMeters':tolerance,'shortLeafMeters':minimum})
            print(json.dumps(reports[-1]),flush=True)
        values={'schemaVersion':2,'riverLevels':reports,'riverLODNotes':'合流点・河口・取得範囲端を保持。短い末端の枝の省略は長さによる近似で河川等級ではない。元詳細データは features に保持。'}
        db.executemany('INSERT OR REPLACE INTO metadata VALUES (?,?)',[(k,json.dumps(v,ensure_ascii=False)) for k,v in values.items()])
        db.commit()
        assert db.execute('PRAGMA quick_check').fetchone()[0]=='ok'
    temp.replace(output)
    packed=output.with_suffix(output.suffix+'.gz')
    with output.open('rb') as src,packed.open('wb') as dst,gzip.GzipFile(fileobj=dst,mode='wb',mtime=0) as gz:
        import shutil
        shutil.copyfileobj(src,gz)
    report={'sqliteBytes':output.stat().st_size,'gzipBytes':packed.stat().st_size,'levels':reports}
    output.with_suffix('.report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
    return report


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,default=Path('build/terrain-vectors.sqlite3'))
    parser.add_argument('--output',type=Path,default=Path('build/terrain-vectors-river-lod.sqlite3'))
    args=parser.parse_args();print(json.dumps(generate(args.source,args.output)))
