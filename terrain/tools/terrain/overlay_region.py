"""保存済みの水域・徒歩道・県境を、表示中の地形と同じ座標へ変換する。"""
from contextlib import closing
import gzip
import json
import math
import sqlite3

import numpy as np
from ridges import RADIUS


def water_class(code):
    if 5100<=code<5200:return 'sea'
    if 5230<=code<=5242:return 'lake'
    if 5201<=code<=5212:return 'river'
    return 'unknown'


class Surface:
    def __init__(self,vertices,indices):
        import shapely
        self.vertices=np.asarray(vertices,dtype=float)
        self.faces=np.asarray(indices,dtype=int).reshape(-1,3)
        self.triangles=self.vertices[self.faces]
        self.shapes=shapely.polygons(self.triangles[:,:,:2])
        self.tree=shapely.STRtree(self.shapes)

    def heights(self,xy):
        import shapely
        xy=np.asarray(xy,dtype=float).reshape(-1,2)
        result=np.full(len(xy),np.nan)
        if not len(xy):return result
        points,faces=self.tree.query(shapely.points(xy),predicate='intersects')
        a,b,c=self.triangles[faces].transpose(1,0,2)
        q=xy[points]
        den=(b[:,1]-c[:,1])*(a[:,0]-c[:,0])+(c[:,0]-b[:,0])*(a[:,1]-c[:,1])
        w=((b[:,1]-c[:,1])*(q[:,0]-c[:,0])+(c[:,0]-b[:,0])*(q[:,1]-c[:,1]))/den
        v=((c[:,1]-a[:,1])*(q[:,0]-c[:,0])+(a[:,0]-c[:,0])*(q[:,1]-c[:,1]))/den
        result[points]=w*a[:,2]+v*b[:,2]+(1-w-v)*c[:,2]
        return result


def add_overlays(payload,database,river_level=None,include_lines=True):
    if not payload.get('polygon') or not database.exists():return payload
    import shapely
    from shapely.geometry import shape,box,Polygon,LineString
    from shapely.ops import unary_union
    lon,lat,alt=payload['projectionCenter']
    sx=math.pi/180*RADIUS*math.cos(math.radians(lat));sy=math.pi/180*RADIUS
    def project(x,y,z=None):return (np.asarray(x)-lon)*sx,(np.asarray(y)-lat)*sy
    points=np.asarray(payload['mesh'])
    low=points[:,:2].min(axis=0);high=points[:,:2].max(axis=0)
    clip=box(*low,*high)
    bounds=[low[0]/sx+lon,low[1]/sy+lat,high[0]/sx+lon,high[1]/sy+lat]
    west,south,east,north=bounds
    with closing(sqlite3.connect(database.resolve().as_uri()+'?mode=ro',uri=True)) as db:
        meta={k:json.loads(v) for k,v in db.execute('SELECT key,value FROM metadata')}
        from river_lod import choose_level
        river_level=river_level or choose_level(payload.get('coverage',{}).get('sizeKm',payload['span']/1000),lat)
        broad=payload.get('coverage',{}).get('sizeKm',payload['span']/1000)>100
        lod=bool(meta.get('riverLevels'))
        rows=[] if broad else db.execute('SELECT kind,body FROM features WHERE kind!=? AND west<=? AND east>=? AND south<=? AND north>=?',('river' if lod else '',east,west,north,south)).fetchall()
        if not include_lines:rows=[row for row in rows if row[0] not in ('river','trail','boundary')]
        if lod and include_lines:
            rows+=db.execute("SELECT 'river',body FROM river_lod WHERE level=? AND west<=? AND east>=? AND south<=? AND north>=?",(river_level,east,west,north,south)).fetchall()
    if not rows:
        payload['overlays']={'layers':{k:{'triangles':[],'lines':[]} for k in ['water','trail','boundary']},'counts':{k:0 for k in ['water','river','trail','boundary']},'available':False,'complete':False,'estimatedLakeLevels':0,'riverLevel':river_level,'attribution':meta['attribution'],'bounds':meta['bounds'],'message':'追加レイヤーは取得済みの富士山・富士五湖・駿河湾周辺を拡大すると表示します。'}
        return payload
    surface=Surface(payload['mesh'],payload['indices'])
    groups={'sea':[],'lake':[],'river':[],'unknown':[]};lines={'trail':[],'boundary':[],'river':[]};lake_edges=[]
    counts={'water':0,'trail':0,'boundary':0,'river':0}
    for kind,body in rows:
        item=json.loads(gzip.decompress(body));geometry=shapely.transform(shape(item['geometry']),project,interleaved=False).intersection(clip)
        if kind!='river':geometry=geometry.simplify(max(1,payload['meshStepMeters']*.25),preserve_topology=True)
        if geometry.is_empty:continue
        if kind=='lakeedge':lake_edges.append(geometry);continue
        counts[kind]+=1
        if kind=='water':groups[water_class(item['properties']['vt_code'])].append(geometry)
        else:lines[kind].append(geometry)
    def parts(geometry,wanted):
        if geometry.geom_type in wanted:yield geometry
        elif hasattr(geometry,'geoms'):
            for child in geometry.geoms:yield from parts(child,wanted)
    # WA の「河川・湖池」だけでは湖と断定できない。湖池の水涯線が閉じる面だけを水平化する。
    if lake_edges:
        edges=shapely.set_precision(unary_union(lake_edges),1.)
        lake_faces=list(shapely.polygonize(list(parts(edges,{'LineString'}))).geoms)
        if lake_faces and groups['unknown']:
            unknown=unary_union(groups['unknown'])
            lakes=unary_union(lake_faces).intersection(unknown)
            groups['lake'].append(lakes);groups['unknown']=[unknown.difference(lakes)]
    flat=[];water=[];estimated=0
    for kind,geometries in groups.items():
        if not geometries:continue
        merged=unary_union(geometries)
        for polygon in parts(merged,{'Polygon'}):
            if kind=='sea':height=-alt
            elif kind=='lake':
                # 水面標高がないため、岸の地形から推定する。元データの実測水位ではない。
                shore=shapely.segmentize(polygon.exterior,50)
                heights=surface.heights(np.asarray(shore.coords))
                valid=heights[np.isfinite(heights)]
                if not len(valid):continue
                height=float(np.median(valid));estimated+=1
            else:height=None
            if height is not None:flat.append((polygon,height))
            for triangle in shapely.constrained_delaunay_triangles(polygon).geoms:
                xy=np.asarray(triangle.exterior.coords)[:3,:2]
                heights=np.full(3,height) if height is not None else surface.heights(xy)
                if np.isfinite(heights).all():water.extend(np.column_stack([xy,heights+.5]).ravel().tolist())
    # 湖・海の内部の地形面を切り抜く。岸では同じ水面標高を使う。
    if flat:
        wet=unary_union([g for g,h in flat]);wet_tree=shapely.STRtree([g for g,h in flat])
        affected=surface.tree.query(wet,predicate='intersects')
        changed=set(map(int,affected));vertices=payload['mesh'];indices=[]
        for i,face in enumerate(surface.faces):
            if i not in changed:indices.extend(face.tolist());continue
            remaining=surface.shapes[i].difference(wet)
            for polygon in parts(remaining,{'Polygon'}):
                for triangle in shapely.constrained_delaunay_triangles(polygon).geoms:
                    xy=np.asarray(triangle.exterior.coords)[:3,:2];heights=surface.heights(xy)
                    for j,p in enumerate(xy):
                        candidates=wet_tree.query(shapely.Point(p).buffer(.01))
                        for k in candidates:
                            if flat[k][0].boundary.distance(shapely.Point(p))<.01:heights[j]=flat[k][1];break
                    if not np.isfinite(heights).all():continue
                    start=len(vertices);vertices.extend(np.column_stack([xy,heights]).tolist());indices.extend([start,start+1,start+2])
        payload['landIndices']=indices
    layers={'water':{'triangles':water,'lines':[]},'trail':{'triangles':[],'lines':[]},'boundary':{'triangles':[],'lines':[]}}
    for kind,geometries in lines.items():
        target=layers['water' if kind=='river' else kind]['lines']
        dash={'trail':35,'boundary':150,'river':0}[kind]
        for geometry in geometries:
            for line in parts(geometry,{'LineString'}):
                if kind=='river':
                    # 画面の約 2 px 間隔で標高を補間する。保存済みの折れ点は消さない。
                    spacing=max(15,payload.get('coverage',{}).get('sizeKm',payload['span']/1000)*1000/900)
                    xy=np.asarray(shapely.segmentize(line,spacing).coords)
                    heights=surface.heights(xy);points=np.column_stack([xy,heights+1.5])
                    valid=np.isfinite(points).all(axis=1)
                    pairs=np.stack([points[:-1],points[1:]],axis=1)[valid[:-1]&valid[1:]]
                    target.extend(pairs.ravel().tolist());continue
                distance=0
                # 地形の三角形と同じ面を補間し、欠損で線を切る。
                while distance<line.length:
                    end=min(line.length,distance+(dash if dash else 50))
                    segment=LineString([line.interpolate(t) for t in np.linspace(distance,end,max(2,math.ceil((end-distance)/15)+1))])
                    xy=np.asarray(segment.coords);heights=surface.heights(xy)
                    points=np.column_stack([xy,heights+1.5])
                    for a,b in zip(points,points[1:]):
                        if np.isfinite(a).all() and np.isfinite(b).all():target.extend([*a,*b])
                    distance=end+(dash*.55 if dash else 0)
    saved=meta['bounds'];complete=bool(west>=saved[0] and south>=saved[1] and east<=saved[2] and north<=saved[3])
    payload['overlays']={'layers':layers,'counts':counts,'available':bool(rows),'complete':complete,'estimatedLakeLevels':estimated,'riverLevel':river_level,'attribution':meta['attribution'],'bounds':saved}
    return payload
