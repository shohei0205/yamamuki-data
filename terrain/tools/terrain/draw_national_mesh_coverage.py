"""全国メッシュの実際の保存タイル配置を地図へ描く。"""
import json,sqlite3,sys
from pathlib import Path
import numpy as np
from PIL import Image,ImageDraw,ImageFont
from shapely import wkb
from shapely.ops import transform
from national_independent_mesh import ROOT,project,R

OUT=ROOT/'build/national-mesh-coverage.png'
FONT='C:/Windows/Fonts/meiryo.ttc'

def main():
    with sqlite3.connect((ROOT/'build/terrain-error-national-base.sqlite3').as_uri()+'?mode=ro',uri=True) as db:
        rows=db.execute('SELECT x,y,state FROM jobs WHERE error=800 ORDER BY y,x').fetchall()
    tiles=[(x,y) for x,y,state in rows if state==1];empty=[(x,y) for x,y,state in rows if state==2]
    coast=wkb.loads((ROOT/'build/terrain-coast-source.union.wkb').read_bytes())
    def mercator_to_grid(x,y,z=None):
        return project(np.degrees(np.asarray(x)/R),np.degrees(2*np.arctan(np.exp(np.asarray(y)/R))-np.pi/2))
    land=transform(mercator_to_grid,coast)
    canvas=Image.new('RGB',(2300,1750),'#f5f7fa');d=ImageDraw.Draw(canvas)
    def text(x,y,s,size=24,fill='#20334b'):
        d.text((x,y),s,font=ImageFont.truetype(FONT,size),fill=fill)
    text(50,25,'全国メッシュ：保存タイルのカバー範囲',44)
    text(52,90,'8 km × 8 km の正方形タイル／標高あり 7,355 枚／元データなし 379 枚',27)
    text(52,132,'全国基本生成の実配置。海岸で切り抜かず、タイル内の標高欠損は仮の 0 m。',24)

    def panel(box,bounds,title,grid=True):
        left,top,right,bottom=box;x0,y0,x1,y1=bounds
        scale=min((right-left)/(x1-x0),(bottom-top)/(y1-y0))
        ox=(left+right-(x1-x0)*scale)/2;oy=(top+bottom+(y1-y0)*scale)/2
        def xy(x,y):return ox+(np.asarray(x)-x0)*scale,oy-(np.asarray(y)-y0)*scale
        d.rectangle(box,fill='#e7f0f8',outline='#acbbc9',width=2)
        # 海岸形状は背景の位置確認だけに使う。
        clipped=land.intersection(__import__('shapely.geometry',fromlist=['box']).box(*bounds))
        polygons=list(clipped.geoms) if hasattr(clipped,'geoms') else [clipped]
        for polygon in polygons:
            if polygon.geom_type!='Polygon':continue
            a=np.asarray(polygon.exterior.coords);xx,yy=xy(a[:,0],a[:,1]);d.polygon(list(zip(xx,yy)),fill='#ccd8bf')
        for entries,fill,outline in [(empty,None,'#c2cbd4'),(tiles,'#dbe9f4','#3a79b5')]:
            for x,y in entries:
                a,b=x*8000,y*8000
                if a+8000<x0 or a>x1 or b+8000<y0 or b>y1:continue
                lo,hi=xy(max(a,x0),min(b+8000,y1));rr,bb=xy(min(a+8000,x1),max(b,y0))
                if fill:d.rectangle((lo,hi,rr,bb),fill=fill)
                d.rectangle((lo,hi,rr,bb),outline=outline,width=1)
        # 陸地の外周線を背景に重ね、青いタイルの海への張り出しも確認できる。
        for polygon in polygons:
            if polygon.geom_type!='Polygon':continue
            a=np.asarray(polygon.exterior.coords);xx,yy=xy(a[:,0],a[:,1]);d.line(list(zip(xx,yy)),fill='#6d7f69',width=1)
        text(left+12,top+10,title,24)
        return xy

    xs=[x*8000 for x,y,_ in rows];ys=[y*8000 for x,y,_ in rows]
    bounds=(min(xs)-50000,min(ys)-50000,max(xs)+58000,max(ys)+58000)
    world=panel((45,195,1640,1590),bounds,'全国：離島を含む現在の目録')
    for name,lon,lat in [('北海道',143,43.8),('本州',138,39),('四国',133.6,33.3),('九州',130.1,32.4),('南西諸島',126.4,26.2),('小笠原諸島',143.2,26.7)]:
        x,y=world(*project(lon,lat));text(x+8,y,name,21)
    regions=[('富士山',138.7274,35.3606),('槍ヶ岳',137.6476,36.3419),('筑波山',140.1067,36.2254)]
    for index,(name,lon,lat) in enumerate(regions):
        cx,cy=map(float,project(lon,lat));top=195+index*465
        convert=panel((1680,top,2255,top+430),(cx-64000,cy-64000,cx+64000,cy+64000),name+'周辺：128 km 四方')
        x,y=convert(cx,cy);d.ellipse((x-6,y-6,x+6,y+6),fill='#c34431');text(x+10,y+3,name,20)
        wx,wy=world(cx,cy);d.ellipse((wx-6,wy-6,wx+6,wy+6),fill='#c34431')
    d.rectangle((55,1620,85,1648),fill='#dbe9f4',outline='#3a79b5');text(100,1616,'標高あり：8 km タイル（海も含む矩形）',23)
    d.rectangle((750,1620,780,1648),outline='#c2cbd4',width=2);text(795,1616,'標高なし：生成対象外',23)
    text(55,1670,'背景の海岸線は位置の参考のみ。メッシュに海岸線データは格納しない。',23)
    text(55,1705,'球面ランベルト正角円錐図法／投影座標上の一辺 8 km。描画LOD・大きな保存タイルへの統合は別途。',20)
    canvas.save(OUT)
    (OUT.with_suffix('.json')).write_text(json.dumps(dict(tileWidthMeters=8000,withElevation=len(tiles),withoutElevation=len(empty),candidateTiles=len(rows),projectedTileAreaKm2=64,totalProjectedAreaKm2=len(tiles)*64,coastlineInMesh=False),ensure_ascii=False,indent=2),encoding='utf-8')
    print(OUT)

if __name__=='__main__':main()
