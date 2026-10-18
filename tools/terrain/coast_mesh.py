"""元の海域ポリゴンを簡略化し、各レベルの地形を海岸線に沿って分割する。"""
import argparse
from contextlib import closing
import gzip
import hashlib
import json
import math
from pathlib import Path
import shutil
import sqlite3
import time
import numpy as np
import shapely
from shapely.geometry import box
from overlay_region import Surface
from polygon_dem import decode_mesh,encode_coast_mesh,read_grid,read_tile
from coastline_source import WORLD
TOLERANCES={4:250.,5:160.,6:100.,7:65.,8:40.,9:20.,10:10.,11:5.}


def polygon_parts(geometry):
    if geometry.geom_type=='Polygon':return [geometry]
    if hasattr(geometry,'geoms'):return [p for child in geometry.geoms for p in polygon_parts(child)]
    return []


def simplify_land(land,tolerance):
    # 島と周囲の海を別々に簡略化すると、海の穴が島を覆う場合がある。
    # 全ポリゴンを一つの被覆として簡略化し、包含関係を保つ。
    parts=polygon_parts(land)
    simplified=shapely.union_all(shapely.coverage_simplify(parts,tolerance))
    if not simplified.is_valid:raise ValueError('簡略化した輪郭が不正です')
    after=polygon_parts(simplified)
    if len(after)!=len(parts) or sum(len(p.interiors) for p in after)!=sum(len(p.interiors) for p in parts):raise ValueError(f'簡略化で島や海の穴の数が変わりました: polygons {len(parts)}->{len(after)}, holes {sum(len(p.interiors) for p in parts)}->{sum(len(p.interiors) for p in after)}')
    return simplified


def coast_segments(land):
    segments=[]
    for polygon in polygon_parts(land):
        for ring in [polygon.exterior,*polygon.interiors]:
            coords=np.asarray(ring.coords)
            for a,b in zip(coords[:-1],coords[1:]):
                if any(abs(a[axis]-b[axis])<1e-8 and (abs(a[axis])<1e-8 or abs(a[axis]-256)<1e-8) for axis in (0,1)):continue
                segments.append([a,b])
    return shapely.linestrings(np.asarray(segments).reshape(-1,2,2))


def rebuild(vertices,indices,land,grid=None):
    """海岸の輪郭を境界にして面を張り、内部は元の三角形を保つ。"""
    vertices=np.asarray(vertices,dtype=float);faces=np.asarray(indices,dtype=int).reshape(-1,3)
    source=shapely.polygons(vertices[faces,:2])
    # 既存の面を海岸線で切り、新しい陸地の部分だけ補う。
    shapely.prepare(land)
    inside=shapely.covers(land,source)
    touches=shapely.intersects(land,source)&~inside
    clipped=list(source[inside])
    clipped.extend(shapely.intersection(source[touches],land))
    segments=coast_segments(land)
    existing=shapely.union_all(source)
    extra=land.difference(existing)
    if len(segments):
        shoreline=shapely.union_all(segments);shapely.prepare(shoreline)
        candidates=np.asarray(polygon_parts(extra),dtype=object)
        extras=list(candidates[shapely.intersects(shoreline,candidates)])
    else:extras=[]
    polygons=[p for g in clipped for p in polygon_parts(g)]+extras
    if not polygons:return np.empty((0,3),dtype=np.float32),np.empty((0,3),dtype=np.uint32)
    triangulated=shapely.get_parts(shapely.constrained_delaunay_triangles(np.asarray(polygons,dtype=object)))
    triangulated=triangulated[shapely.get_type_id(triangulated)==3]
    if not len(triangulated):return np.empty((0,3),dtype=np.float32),np.empty((0,3),dtype=np.uint32)
    coords=shapely.get_coordinates(triangulated).reshape(-1,4,2)[:,:3,:]
    xy,inverse=np.unique(np.round(coords.reshape(-1,2),5).astype(np.float32),axis=0,return_inverse=True)
    result_faces=inverse.reshape(-1,3).astype(np.uint32)
    a,b,c=xy[result_faces].transpose(1,0,2)
    area=(b[:,0]-a[:,0])*(c[:,1]-a[:,1])-(b[:,1]-a[:,1])*(c[:,0]-a[:,0])
    result_faces=result_faces[np.abs(area)>1e-10]
    heights=Surface(vertices,faces.ravel()).heights(xy)
    # DEM にない海岸付近の追加の陸地は海抜 0 m とする。内陸の欠損は埋めない。
    heights[~np.isfinite(heights)]=0
    if grid is not None:
        for axis,value,line in [(0,0,grid[:,0]),(0,256,grid[:,-1]),(1,0,grid[0,:]),(1,256,grid[-1,:])]:
            selected=np.abs(xy[:,axis]-value)<1e-7
            valid=np.flatnonzero(np.isfinite(line))
            heights[selected]=np.interp(xy[selected,1-axis],valid,line[valid]) if len(valid) else 0
    if len(segments):
        points,edges=shapely.STRtree(segments).query(shapely.points(xy),predicate='dwithin',distance=3e-5)
        heights[np.unique(points)]=0
    return np.column_stack([xy,heights]).astype(np.float32),result_faces


def reconcile_boundaries(db,z):
    """海岸線がタイル境界に重なる場合、両側の共有標高を海抜 0 m にそろえる。"""
    seen={};conflicts={};tiles=[]
    for x,y,body in db.execute('SELECT x,y,data FROM tiles WHERE z=?',(z,)):
        points,faces=decode_mesh(body);tiles.append((x,y,points,faces))
        edge=points[(points[:,0]==0)|(points[:,0]==256)|(points[:,1]==0)|(points[:,1]==256)]
        for vx,vy,h in edge:
            key=(round(x*256+float(vx),5),round(y*256+float(vy),5));h=float(h)
            if key in seen and seen[key]!=h:conflicts.setdefault(key,{seen[key]}).add(h)
            seen[key]=h
    for heights in conflicts.values():
        if 0 not in heights:raise ValueError('海岸以外の共有標高が一致しません')
    changed=0
    for x,y,points,faces in tiles:
        edited=False
        for index in np.flatnonzero((points[:,0]==0)|(points[:,0]==256)|(points[:,1]==0)|(points[:,1]==256)):
            vx,vy,h=points[index];key=(round(x*256+float(vx),5),round(y*256+float(vy),5))
            if key in conflicts and h!=0:points[index,2]=0;edited=True
        if edited:
            db.execute('UPDATE tiles SET data=? WHERE z=? AND x=? AND y=?',(encode_coast_mesh(points,faces),z,x,y));changed+=1
    return {'sharedCoastVertices':len(conflicts),'tilesAdjusted':changed}


def build(source,coast,grids,output,levels=None):
    with closing(sqlite3.connect(source.resolve().as_uri()+'?mode=ro',uri=True)) as available_db:
        available={z for z, in available_db.execute('SELECT DISTINCT z FROM tiles')}
    selected={z:TOLERANCES[z] for z in (TOLERANCES if levels is None else levels) if z in available}
    if levels is not None and set(levels)!=set(selected):raise ValueError('指定したレベルの標高データがありません')
    if not selected:raise ValueError('海岸線を生成するレベルがありません')
    if output.exists():raise ValueError('出力先が既にあります')
    with closing(sqlite3.connect(coast.resolve().as_uri()+'?mode=ro',uri=True)) as db:
        cache=coast.with_suffix('.union.wkb');identity=coast.with_suffix('.union.json')
        signature={'bytes':coast.stat().st_size,'modifiedNs':coast.stat().st_mtime_ns}
        if cache.exists() and identity.exists() and json.loads(identity.read_text())==signature:
            land=shapely.from_wkb(cache.read_bytes())
        else:
            land=shapely.union_all([shapely.from_wkb(data) for data, in db.execute('SELECT land FROM tiles')],grid_size=.01)
            cache.write_bytes(shapely.to_wkb(land));identity.write_text(json.dumps(signature))
        print('coastline union complete',len(polygon_parts(land)),flush=True)
        coast_meta={k:json.loads(v) for k,v in db.execute('SELECT key,value FROM metadata')}
    if not land.is_valid:raise ValueError('元の陸地ポリゴンが不正です')
    temporary=output.with_suffix('.part.sqlite3')
    if temporary.exists():raise ValueError('未完成の出力があります')
    shutil.copyfile(source,temporary);reports=[]
    with closing(sqlite3.connect(temporary)) as dest,closing(sqlite3.connect(grids.resolve().as_uri()+'?mode=ro',uri=True)) as grid_db:
        for z,tolerance in selected.items():
            start=time.perf_counter();simple=simplify_land(land,tolerance)
            if not simple.is_valid:raise ValueError('簡略化した輪郭が不正です')
            if len(polygon_parts(simple))!=len(polygon_parts(land)):raise ValueError('簡略化で島の数が変わりました')
            resolution=WORLD/(256*2**z);offset=(1-2**(z-14))/2
            coords=dest.execute('SELECT x,y FROM tiles WHERE z=? ORDER BY x,y',(z,)).fetchall();changed=0;removed=0
            for i,(x,y) in enumerate(coords):
                left=(x*256+offset)*resolution-WORLD/2;top=WORLD/2-(y*256+offset)*resolution
                boundary=box(left,top-256*resolution,left+256*resolution,top)
                local=simple.intersection(boundary)
                if local.is_empty:
                    dest.execute('DELETE FROM tiles WHERE z=? AND x=? AND y=?',(z,x,y));removed+=1;continue
                if local.covers(boundary):continue
                local=shapely.transform(local,lambda a:np.column_stack([(a[:,0]-left)/resolution,(top-a[:,1])/resolution]))
                row=dest.execute('SELECT data FROM tiles WHERE z=? AND x=? AND y=?',(z,x,y)).fetchone()
                print('rebuild',z,x,y,flush=True)
                points,faces=decode_mesh(row[0]);grid=read_grid(grid_db,z,x,y)
                points,faces=rebuild(points,faces,local,grid)
                if not len(faces):dest.execute('DELETE FROM tiles WHERE z=? AND x=? AND y=?',(z,x,y));removed+=1;continue
                body=encode_coast_mesh(points,faces);restored,triangles=decode_mesh(body)
                np.testing.assert_array_equal(restored[:,:2],points[:,:2]);np.testing.assert_array_equal(triangles,faces)
                if np.max(np.abs(restored[:,2]-points[:,2]))>2.501:raise ValueError('海岸メッシュの標高丸め誤差が大きすぎます')
                dest.execute('UPDATE tiles SET data=? WHERE z=? AND x=? AND y=?',(body,z,x,y));changed+=1
                if changed%16==0:dest.commit();print('level',z,'processed',i+1,'/',len(coords),'coast tiles',changed,flush=True)
            stitching=reconcile_boundaries(dest,z)
            dest.commit();read_tile.cache_clear()
            count,total=dest.execute('SELECT count(*),sum(length(data)) FROM tiles WHERE z=?',(z,)).fetchone()
            report={'zoom':z,'toleranceMeters':tolerance,'tiles':count,'bytes':total,'coastTiles':changed,'removedEmptyTiles':removed,'boundaryStitching':stitching,'seconds':time.perf_counter()-start};reports.append(report);print(json.dumps(report),flush=True)
        meta={k:json.loads(v) for k,v in dest.execute('SELECT key,value FROM metadata')}
        meta.update(schemaVersion=2,encoding='YMP2+YMP4',coastlineToleranceMeters={str(z):v for z,v in selected.items()},coastlineSource=coast_meta,coastlineNotes='海域ポリゴンの境界を簡略化して三角形を分割。追加した海岸付近の陸地で DEM がない部分と海岸線は海抜 0 m。内陸の DEM 欠損は保持。海岸の分割による標高変化と 5 m 丸めの誤差は元の meshErrorBounds と別。')
        dest.executemany('INSERT OR REPLACE INTO metadata VALUES (?,?)',[(k,json.dumps(v,ensure_ascii=False)) for k,v in meta.items()]);dest.commit()
        assert dest.execute('PRAGMA quick_check').fetchone()[0]=='ok'
    temporary.replace(output)
    with output.open('rb') as src,gzip.open(str(output)+'.gz','wb',compresslevel=6) as dest:shutil.copyfileobj(src,dest)
    report={'levelReports':reports,'sqliteBytes':output.stat().st_size,'gzipBytes':Path(str(output)+'.gz').stat().st_size,**meta}
    output.with_suffix('.report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8');print(json.dumps({'sqliteBytes':report['sqliteBytes'],'gzipBytes':report['gzipBytes']}),flush=True)
    return report

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,default=Path('build/terrain-mesh.sqlite3'))
    parser.add_argument('--coast',type=Path,default=Path('build/terrain-coast-source.sqlite3'))
    parser.add_argument('--grids',type=Path,default=Path('build/terrain-lod.sqlite3'))
    parser.add_argument('--output',type=Path,default=Path('build/terrain-coast-mesh.sqlite3'))
    parser.add_argument('--levels',type=int,nargs='+',choices=list(TOLERANCES))
    args=parser.parse_args();build(args.source,args.coast,args.grids,args.output,args.levels)
