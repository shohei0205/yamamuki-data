"""保存済みの RIV1/WAT1 を無加工で中継する。"""
from contextlib import closing
import sqlite3
from sqlite_relay import pack_rows


def relay(database,query):
    try:
        z=int(query['z'][0]);x0=int(query['x0'][0]);x1=int(query['x1'][0]);y0=int(query['y0'][0]);y1=int(query['y1'][0])
    except (KeyError,ValueError,TypeError):
        raise ValueError('水域タイルの範囲が不正です')
    if z not in (10,11,12) or not 0<=x0<=x1<2**z or not 0<=y0<=y1<2**z or (x1-x0+1)*(y1-y0+1)>65536:
        raise ValueError('水域タイルの範囲が不正です')
    try:
        sea_z=int(query.get('seaZ',[z])[0]);river_z=int(query.get('riverZ',[z])[0])
    except (ValueError,TypeError):
        raise ValueError('水域レベルが不正です')
    if sea_z not in (10,11,12) or river_z not in (10,11,12):
        raise ValueError('水域レベルが不正です')
    rows=[]
    with closing(sqlite3.connect(database.resolve().as_uri()+'?mode=ro',uri=True)) as db:
        for layer,level in [('rivers',river_z),('water',river_z),('sea',sea_z)]:
            ratio=2**(level-z)
            bounds=(int(x0*ratio),int((x1+1)*ratio-1e-8),int(y0*ratio),int((y1+1)*ratio-1e-8))
            rows.extend((dict(layer=layer,z=level,x=x,y=y),body) for x,y,body in db.execute(
                'SELECT x,y,data FROM tiles WHERE layer=? AND z=? AND x BETWEEN ? AND ? AND y BETWEEN ? AND ? ORDER BY x,y',(layer,level,*bounds)))
    return pack_rows(rows)
