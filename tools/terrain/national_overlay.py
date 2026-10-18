"""全国の線データを地形とは独立した縮尺で読み込み、表示座標へ変換する。"""
from contextlib import closing
import json
import math
import sqlite3

import numpy as np
import shapely
from shapely.geometry import LineString
from ridges import RADIUS
from national_lines import LEVELS,decode_lines,line_parts
from overlay_region import Surface


def choose_level(size,latitude):
    desired=math.floor(math.log2(40075016.686*math.cos(math.radians(latitude))*1800/(max(2,size)*1000*256)))
    return max((z for z in LEVELS if z<=desired),default=4)


def add_national_lines(payload,database):
    if not payload.get('polygon') or not database.exists():return payload
    lon,lat,alt=payload['projectionCenter'];sx=math.pi/180*RADIUS*math.cos(math.radians(lat));sy=math.pi/180*RADIUS
    points=np.asarray(payload['mesh']);low=points[:,:2].min(axis=0);high=points[:,:2].max(axis=0)
    west,south=low/[sx,sy]+[lon,lat];east,north=high/[sx,sy]+[lon,lat]
    size=payload.get('coverage',{}).get('sizeKm',payload['span']/1000);level=choose_level(size,lat)
    # 粗いタイル全域ではなく、要求した範囲内だけ線を取得する。
    coverage=payload.get('coverage',{})
    if 'latitude' in coverage and 'longitude' in coverage:
        delta=math.degrees(size*500/RADIUS)
        request_lon=coverage['longitude'];request_lat=coverage['latitude']
        west=max(west,request_lon-delta/math.cos(math.radians(request_lat)))
        east=min(east,request_lon+delta/math.cos(math.radians(request_lat)))
        south=max(south,request_lat-delta);north=min(north,request_lat+delta)
        low=np.maximum(low,[(west-lon)*sx,(south-lat)*sy])
        high=np.minimum(high,[(east-lon)*sx,(north-lat)*sy])
    with closing(sqlite3.connect(database.resolve().as_uri()+'?mode=ro',uri=True)) as db:
        meta={k:json.loads(v) for k,v in db.execute('SELECT key,value FROM metadata')}
        if not meta.get('complete'):return payload
        rows=db.execute('SELECT t.kind,t.body FROM line_tiles t JOIN line_bounds b ON b.id=t.id WHERE t.level=? AND b.west<=? AND b.east>=? AND b.south<=? AND b.north>=?',(level,east,west,north,south)).fetchall()
    mercator_bounds=[RADIUS*math.radians(west),RADIUS*math.log(math.tan(math.pi/4+math.radians(max(-85,south))/2)),RADIUS*math.radians(east),RADIUS*math.log(math.tan(math.pi/4+math.radians(min(85,north))/2))]
    overlays=payload.setdefault('overlays',{'layers':{},'counts':{},'attribution':meta['attribution'],'estimatedLakeLevels':0})
    for kind in ['water','trail','boundary']:overlays['layers'].setdefault(kind,{'lines':[],'triangles':[]})
    # 地域限定データの線を重ねない。水域の面だけを保持する。
    for kind in ['water','trail','boundary']:overlays['layers'][kind]['lines']=[]
    for kind in ['river','trail','boundary']:overlays['counts'][kind]=0
    overlays['counts'].setdefault('water',0)
    clip=shapely.box(*low,*high);surface=Surface(payload['mesh'],payload['indices'])
    spacing=max(15,size*1000/900)
    for kind,body in rows:
        for xy in decode_lines(body):
            if xy[:,0].max()<mercator_bounds[0] or xy[:,0].min()>mercator_bounds[2] or xy[:,1].max()<mercator_bounds[1] or xy[:,1].min()>mercator_bounds[3]:continue
            longitudes=np.degrees(xy[:,0]/RADIUS)
            latitudes=np.degrees(2*np.arctan(np.exp(xy[:,1]/RADIUS))-math.pi/2)
            projected=np.column_stack([(longitudes-lon)*sx,(latitudes-lat)*sy])
            geometry=LineString(projected).intersection(clip)
            for line in line_parts(geometry):
                if line.length==0:continue
                overlays['counts'][kind]+=1
                xy=np.asarray(shapely.segmentize(line,spacing).coords)
                heights=surface.heights(xy);xyz=np.column_stack([xy,heights+1.5]);valid=np.isfinite(xyz).all(axis=1)
                if kind=='river':keep=np.ones(len(xyz)-1,dtype=bool)
                else:
                    # 広域でも破線の区間数を画面解像度に合わせる。
                    distance=np.r_[0,np.cumsum(np.linalg.norm(np.diff(xy,axis=0),axis=1))]
                    dash=max(35 if kind=='trail' else 150,spacing*3)
                    keep=(np.floor((distance[:-1]+distance[1:])*.5/dash).astype(int)%2)==0
                pairs=np.stack([xyz[:-1],xyz[1:]],axis=1)[valid[:-1]&valid[1:]&keep]
                overlays['layers']['water' if kind=='river' else kind]['lines'].extend(pairs.ravel().tolist())
    overlays.update({'riverLevel':level,'lineLevel':level,'lineLevels':list(LEVELS),'nationalLines':True,'available':bool(rows) or overlays.get('available',False),'bounds':meta['bounds'],'attribution':meta['attribution'],'lineComplete':True,'waterComplete':overlays.get('complete',False)})
    overlays.pop('message',None)
    payload['overlays']=overlays
    return payload
