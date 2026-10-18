"""SQLite の適応ポリゴンを、選択した地域だけ読み込んで表示する。"""
from contextlib import closing
import json
import math
import sqlite3
import numpy as np
from polygon_dem import decode_mesh
from ridges import tile_position,lon_lat,RADIUS


def payload(database,latitude,longitude,size_km,requested_zoom,view=None,outline=False):
    with closing(sqlite3.connect(database.resolve().as_uri()+'?mode=ro',uri=True)) as db:
        meta={k:json.loads(v) for k,v in db.execute('SELECT key,value FROM metadata')}
        from outline_cache import load
        outline_offsets,outline_levels=load(database) if outline else ({},set())
        levels=sorted(meta['levels'])
        zoom=max((z for z in levels if z<=requested_zoom),default=levels[0])
        spatial=lambda level:meta.get('geometryZoomByLevel',{}).get(str(level),level)
        corners=meta.get('pixelOrigin')=='highest-grid-corners'
        delta=math.degrees(size_km*500/RADIUS)
        x0,y0=tile_position(max(-179.999,longitude-delta/math.cos(math.radians(latitude))),min(85,latitude+delta),spatial(zoom))
        x1,y1=tile_position(min(179.999,longitude+delta/math.cos(math.radians(latitude))),max(-85,latitude-delta),spatial(zoom))
        x0,y0,x1,y1=math.floor(x0),math.floor(y0),math.floor(x1),math.floor(y1)
        offset=0 if corners else (1-2**(zoom-14))/2
        center_lon,center_lat=lon_lat((x0+x1+1)*128+offset,(y0+y1+1)*128+offset,spatial(zoom))
        selected=[(zoom,x,y) for y in range(y0,y1+1) for x in range(x0,x1+1)]
        if view and view[0]>=45:
            tilt,bearing,distance=view
            angle=math.radians(bearing); inclination=math.sin(math.radians(tilt))
            # 投影後の大きさに応じて、手前だけ子タイルへ分割する。
            def refine(z,x,y):
                scale=2**(spatial(zoom)-spatial(z))
                if (x+1)*scale<=x0 or x*scale>=x1+1 or (y+1)*scale<=y0 or y*scale>=y1+1:return []
                depths=[]
                for px in (max(x*scale,x0),min((x+1)*scale,x1+1)):
                    for py in (max(y*scale,y0),min((y+1)*scale,y1+1)):
                        lon,lat=lon_lat(px*256,py*256,spatial(zoom))
                        east=math.radians(lon-longitude)*RADIUS*math.cos(math.radians(latitude))
                        north=math.radians(lat-latitude)*RADIUS
                        depths.append((math.sin(angle)*east+math.cos(angle)*north)*inclination)
                depth=min(depths)
                # 手前の追加の詳細は、カメラ付近の小さな範囲に限定する。
                camera_east=-distance*inclination*math.sin(angle)
                camera_north=-distance*inclination*math.cos(angle)
                lon0,lat0=lon_lat(max(x*scale,x0)*256,min((y+1)*scale,y1+1)*256,spatial(zoom))
                lon1,lat1=lon_lat(min((x+1)*scale,x1+1)*256,max(y*scale,y0)*256,spatial(zoom))
                west=math.radians(lon0-longitude)*RADIUS*math.cos(math.radians(latitude))
                east=math.radians(lon1-longitude)*RADIUS*math.cos(math.radians(latitude))
                south=math.radians(lat0-latitude)*RADIUS
                north=math.radians(lat1-latitude)*RADIUS
                near_distance=math.hypot(camera_east-min(east,max(west,camera_east)),camera_north-min(north,max(south,camera_north)))
                ratio=max(.15,(distance+depth)/distance)
                target=levels[max(0,levels.index(zoom)-(2 if ratio>3.5 else 1 if ratio>1.4 else 0))]
                if near_distance<min(4000,size_km*100):target=levels[min(len(levels)-1,levels.index(zoom)+1)]
                target=max((level for level in levels if level<=target),default=levels[0])
                child=next((level for level in levels if level>z),None)
                if child is not None and z<target:
                    factor=2**(spatial(child)-spatial(z))
                    return [tile for cy in range(y*factor,(y+1)*factor) for cx in range(x*factor,(x+1)*factor) for tile in refine(child,cx,cy)]
                return [(z,x,y)]
            base=levels[max(0,levels.index(zoom)-2)];factor=2**(spatial(zoom)-spatial(base))
            selected=[tile for y in range(y0//factor,y1//factor+1) for x in range(x0//factor,x1//factor+1) for tile in refine(base,x,y)]
        vertices=[];faces=[];lookup={};saved=0;missing=0;counts={}
        mixed=len({tile[0] for tile in selected})>1
        for tile_zoom,x,y in selected:
                offset=(1-2**(tile_zoom-14))/2 if not mixed and not corners else 0
                row=db.execute('SELECT data FROM tiles WHERE z=? AND x=? AND y=?',(tile_zoom,x,y)).fetchone()
                if row is None:missing+=1;continue
                original_points,indices=decode_mesh(row[0]);mapping=[]
                points=original_points.copy()
                adjustments=outline_offsets.get((tile_zoom,x,y))
                if adjustments is not None:
                    positions=adjustments[:,0].astype(np.int64)
                    if np.any(positions<0) or np.any(positions>=len(points)):raise ValueError('輪郭の頂点番号が不正です')
                    points[positions]+=adjustments[:,1:]

                for point_index,(vx,vy,height) in enumerate(points):
                    key=(tile_zoom,round(x*256+float(original_points[point_index,0]),5),round(y*256+float(original_points[point_index,1]),5))
                    if key not in lookup:
                        lookup[key]=len(vertices)
                        lon,lat=lon_lat(x*256+float(vx)+offset,y*256+float(vy)+offset,spatial(tile_zoom))
                        vertices.append([(lon-center_lon)*math.pi/180*RADIUS*math.cos(math.radians(center_lat)),(lat-center_lat)*math.pi/180*RADIUS,float(height)])
                    elif abs(vertices[lookup[key]][2]-height)>.001:raise ValueError('タイル境界の標高が一致しません')
                    mapping.append(lookup[key])
                if indices.size:faces.extend(np.asarray(mapping,dtype=np.uint32)[indices].ravel().tolist())
                if mixed and indices.size:
                    # レベル間で標高と頂点数が異なる境界を、下向きの裾でふさぐ。
                    edges={}
                    for face in indices:
                        for i,j in zip(face,(face[1],face[2],face[0])):
                            edge=tuple(sorted((int(i),int(j))));edges[edge]=edges.get(edge,0)+1
                    drop=max(50,meta['meshErrorBounds'][str(max((level for level in levels if level<tile_zoom),default=levels[0]))]*2)
                    for (i,j),count in edges.items():
                        if count!=1:continue
                        a,b=mapping[i],mapping[j]
                        if not ((original_points[i][0]==original_points[j][0] and original_points[i][0] in (0,256)) or (original_points[i][1]==original_points[j][1] and original_points[i][1] in (0,256))):continue
                        c=len(vertices);vertices.extend([[*vertices[a][:2],vertices[a][2]-drop],[*vertices[b][:2],vertices[b][2]-drop]])
                        faces.extend([a,b,c,b,c+1,c])
                counts[str(tile_zoom)]=counts.get(str(tile_zoom),0)+1
                saved+=1
        if not faces:raise ValueError('この範囲には表示できるポリゴンがありません')
        center_height=float(np.median([v[2] for v in vertices]))
        for v in vertices:v[2]-=center_height
        meters=2*math.pi*RADIUS*math.cos(math.radians(center_lat))/(256*2**spatial(zoom))
        result={'polygon':True,'mesh':vertices,'indices':faces,'meshStepMeters':meters,'methods':{},
                'projectionCenter':[center_lon,center_lat,center_height],
                'span':max(x1-x0+1,y1-y0+1)*256*meters,
                'coverage':{'savedTiles':saved,'missingTiles':missing,'latitude':latitude,'longitude':longitude,'sizeKm':size_km,
                            'validSamples':len(vertices),'totalSamples':len(vertices),'heightQuantizationMeters':5,'zoom':zoom,
                            'levels':counts,'geometryZoomByLevel':meta.get('geometryZoomByLevel'),'pixelOrigin':meta.get('pixelOrigin'),'viewLOD':list(view) if view else None,'overview':zoom<14,'polygon':True,'meshErrorMeters':meta['meshErrorBounds'][str(zoom)]}}

        if meta.get('coastlineToleranceMeters') and str(zoom) in meta['coastlineToleranceMeters']:
            result['coverage'].update(outlineZoom=zoom,outlineVector=True,outlineToleranceMeters=meta['coastlineToleranceMeters'][str(zoom)])
        if outline and zoom in outline_levels:
            result['coverage'].update(outlineZoom=zoom,outlineTiles=saved,outlineSmoothed=True,outlinePrecomputed=True)
        return result


def refine_outline(result,database,latitude,longitude,size_km):
    """選択中のレベルの陸地外周を平滑化する。"""
    from overlay_region import Surface
    from scipy.spatial import cKDTree
    fine=result.copy()
    old=np.asarray(result['mesh'],dtype=float)
    points=smooth_outline(fine['mesh'],fine['indices'],fine.get('meshStepMeters',1))
    lon,lat,alt=result['projectionCenter'];flon,flat,falt=fine['projectionCenter']
    sx=math.pi/180*RADIUS*math.cos(math.radians(lat));fsx=math.pi/180*RADIUS*math.cos(math.radians(flat));sy=math.pi/180*RADIUS
    points[:,0]=(points[:,0]/fsx+flon-lon)*sx
    points[:,1]+=(flat-lat)*sy
    heights=Surface(old,result['indices']).heights(points[:,:2])
    missing=~np.isfinite(heights)
    # 粗い海岸線の外にある陸地は、最寄りの粗い標高で補う。
    if missing.any():
        nearest=cKDTree(old[:,:2]).query(points[missing,:2])[1]
        heights[missing]=old[nearest,2]
    points[:,2]=heights
    result['mesh']=points.tolist();result['indices']=fine['indices']
    result['coverage']['outlineZoom']=fine['coverage']['zoom']
    result['coverage']['outlineTiles']=fine['coverage']['savedTiles']
    result['coverage']['outlineSmoothed']=True
    result['coverage']['validSamples']=len(points);result['coverage']['totalSamples']=len(points)


def smooth_outline(vertices,indices,step,passes=4):
    """陸地の外周だけを丸め、面の反転や島・穴の接続変更を防ぐ。"""
    points=np.asarray(vertices,dtype=float).copy()
    faces=np.asarray(indices,dtype=np.int64).reshape(-1,3)
    if not len(faces):return points
    edges=np.concatenate([faces[:,[0,1]],faces[:,[1,2]],faces[:,[2,0]]])
    edges.sort(axis=1);edges,count=np.unique(edges,axis=0,return_counts=True)
    boundary=edges[count==1];neighbors={}
    for a,b in boundary:
        neighbors.setdefault(int(a),[]).append(int(b));neighbors.setdefault(int(b),[]).append(int(a))
    movable=np.array([v for v,adj in neighbors.items() if len(adj)==2],dtype=int)
    if not len(movable):return points
    adjacent=np.array([neighbors[v] for v in movable],dtype=int)
    original=points[:,:2].copy()
    def areas(xy):
        a,b,c=xy[faces].transpose(1,0,2)
        return (b[:,0]-a[:,0])*(c[:,1]-a[:,1])-(b[:,1]-a[:,1])*(c[:,0]-a[:,0])
    initial_area=areas(original)
    for _ in range(passes):
        proposed=points[:,:2].copy()
        proposed[movable]=.5*proposed[movable]+.25*proposed[adjacent].sum(axis=1)
        delta=proposed[movable]-original[movable]
        lengths=np.linalg.norm(delta,axis=1)
        proposed[movable]=original[movable]+delta*np.minimum(1,step*.6/np.maximum(lengths,1e-12))[:,None]
        # 複数の頂点を戻すと別の面にも影響するので、反転がなくなるまで確かめる。
        for _ in range(len(movable)+1):
            area=areas(proposed)
            bad=(area*initial_area<=0)|(np.abs(area)<np.abs(initial_area)*.15)
            bad &= np.abs(initial_area)>1e-8
            if not bad.any():break
            rejected=np.unique(faces[bad])
            if np.array_equal(proposed[rejected],points[rejected,:2]):break
            proposed[rejected]=points[rejected,:2]
        points[:,:2]=proposed
    return points
