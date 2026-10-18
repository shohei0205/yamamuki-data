"""DEM の欠損境界を海域ポリゴンにし、描画用の三角形と WAT1 を保存する。"""
import json
import struct
from pathlib import Path

import contourpy
import numpy as np
from PIL import Image
from scipy import ndimage
import shapely
from shapely.geometry import Polygon, box

ROOT = Path(__file__).resolve().parents[2]

def polygons(geometry):
    if geometry.geom_type == 'Polygon':
        if not geometry.is_empty:
            yield geometry
    elif hasattr(geometry, 'geoms'):
        for part in geometry.geoms:
            yield from polygons(part)

def sea_polygons(valid):
    h, w = valid.shape
    if valid.all():
        return []
    if not valid.any():
        return [box(0, 0, 1, 1)]
    # 周囲を同じ値で補い、表示範囲の外枠も閉じた面として抽出する。
    padded = np.pad(valid, 1, mode='edge')
    signed = ndimage.distance_transform_edt(padded)-ndimage.distance_transform_edt(~padded)
    points, offsets = contourpy.contour_generator(
        x=np.arange(-1, w+1)/(w-1), y=np.arange(-1, h+1)/(h-1),
        z=signed, fill_type='OuterOffset').filled(-1e9, 0)
    out = []
    for coords, starts in zip(points, offsets):
        rings = [coords[a:b] for a,b in zip(starts[:-1], starts[1:])]
        shape = Polygon(rings[0], rings[1:]).simplify(.15/max(w,h), preserve_topology=True)
        out.extend(polygons(shapely.make_valid(shape).intersection(box(0,0,1,1))))
    return out

def encode(areas):
    # 参照実装と同じ 0〜4096 の座標。量子化後の面を描画にも使う。
    wat = bytearray(b'WAT1'+struct.pack('<I',len(areas)))
    triangles = []
    for area in areas:
        rings = [np.rint(np.asarray(r.coords)[:-1]*4096).clip(0,4096).astype(np.uint16)
                 for r in [area.exterior,*area.interiors]]
        wat.extend(struct.pack('<II',1,len(rings)))
        for ring in rings:
            wat.extend(struct.pack('<I',len(ring)))
            wat.extend(ring.astype('<u2').tobytes())
        quantized = shapely.make_valid(Polygon(rings[0]/4096,[r/4096 for r in rings[1:]]))
        for part in polygons(quantized):
            for triangle in shapely.constrained_delaunay_triangles(part).geoms:
                triangles.extend(np.asarray(triangle.exterior.coords)[:3])
    data = np.asarray(triangles,dtype='<f4').reshape(-1,2)
    return bytes(wat), b'SEA1'+struct.pack('<I',len(data))+data.tobytes()

def generate():
    output=ROOT/'build/terrain-sea-vectors'
    output.mkdir(parents=True,exist_ok=True)
    jobs=[]
    for prefix,folder in [('native','terrain-tile-preview'),('region','terrain-heightmap')]:
        for region in ['yari','tsukuba']:
            for level in range(4,15):
                path=ROOT/f'build/{folder}-{region}/levels/{level if prefix=="native" else 14}/height.png'
                if path.exists():
                    jobs.append((f'{prefix}-{region}-L{level}',path,True))
    jobs.append(('national',ROOT/'build/terrain-national-shading/coverage.png',False))
    reports=[]
    cached={}
    for name,path,rgb in jobs:
        image=np.asarray(Image.open(path))
        valid=image[...,2]>127 if rgb else image>127
        if path not in cached:
            areas=sea_polygons(valid)
            wat,mesh=encode(areas)
            cached[path]=(areas,wat,mesh)
        areas,wat,mesh=cached[path]
        (output/f'{name}.wat').write_bytes(wat)
        (output/f'{name}.sea').write_bytes(mesh)
        reports.append(dict(name=name,width=valid.shape[1],height=valid.shape[0],polygons=len(areas),vertices=(len(mesh)-8)//8,watBytes=len(wat),meshBytes=len(mesh),source=str(path.relative_to(ROOT))))
    (output/'manifest.json').write_text(json.dumps(dict(source='DEM の有効・欠損境界。全国は既存の詳細被覆率を 0.5 で判定。',encoding='WAT1; SEA1 float32 UV triangles for WebGL',layers=reports),ensure_ascii=False,indent=2),encoding='utf-8')
    print('海域ベクター生成:',len(reports),'範囲')

if __name__=='__main__':
    generate()
