"""実際の土地被覆分類を、陰影を含まない自然な地表色へ置き換える。"""
import io,json,sqlite3,urllib.request
from contextlib import closing
from pathlib import Path
import numpy as np
import tifffile
from PIL import Image
from national_independent_mesh import ROOT,inverse
OUTPUT=ROOT/'build/terrain-landcover-color.sqlite3'
CACHE=ROOT/'build/terrain-worldcover-source'
SIZES=[128,256,512,1024,2048,4096]
ATTRIBUTION='© ESA WorldCover project 2021 / Contains modified Copernicus Sentinel data (2021) processed by ESA WorldCover consortium'
PALETTE={0:(145,153,142),10:(62,98,58),20:(112,133,77),30:(153,169,99),40:(178,182,117),50:(166,158,149),60:(171,157,139),70:(224,231,233),80:(77,123,146),90:(100,145,115),95:(48,104,86),100:(156,152,111)}
def colorize(classes):
 table=np.zeros((256,3),np.uint8);table[:]=PALETTE[0]
 for code,color in PALETTE.items():table[code]=color
 return table[classes]
def source_path(lat,lon):
 name=f'ESA_WorldCover_10m_2021_v200_N{lat:02d}E{lon:03d}_Map.tif';target=CACHE/name
 if not target.exists():
  print('土地被覆COGを取得',name,flush=True);temporary=target.with_suffix('.download');urllib.request.urlretrieve('https://esa-worldcover.s3.eu-central-1.amazonaws.com/v200/2021/map/'+name,temporary);temporary.replace(target)
 return target
def classes_for_area(area,size=4096):
 keys=area['levels'][0]['keys'];origin=[min(k[1]*1000*2**k[0] for k in keys),min(k[2]*1000*2**k[0] for k in keys)];offset=area['comparisonOffset'];x0=origin[0]+offset[0];y0=origin[1]+offset[1];width=128000.
 x=np.linspace(x0,x0+width,size);ys=np.linspace(y0,y0+width,size)
 lons,lats=inverse(*np.meshgrid(x[[0,-1]],ys[[0,-1]]));lo=int(np.floor(lons.min()/3)*3);hi=int(np.floor(lons.max()/3)*3);bottom=int(np.floor(lats.min()/3)*3);top=int(np.floor(lats.max()/3)*3)
 result=np.zeros((size,size),np.uint8);sources=[]
 for lat in range(bottom,top+1,3):
  for lon in range(lo,hi+1,3):
   path=source_path(lat,lon);sources.append(path.name)
   # 20 mの既成オーバービューを使い、36000角の全解像度配列を展開しない。
   with tifffile.TiffFile(path) as tif:values=tif.pages[1].asarray(maxworkers=2)
   ny,nx=values.shape
   for start in range(0,size,64):
    xx,yy=np.meshgrid(x,ys[start:start+64]);longitude,latitude=inverse(xx,yy);inside=(longitude>=lon)&(longitude<lon+3)&(latitude>=lat)&(latitude<lat+3)
    if not inside.any():continue
    ix=np.floor((longitude[inside]-lon)*nx/3).astype(int).clip(0,nx-1);iy=np.floor((lat+3-latitude[inside])*ny/3).astype(int).clip(0,ny-1)
    result[start:start+64][inside]=values[iy,ix]
   del values
 return result,sources,origin

def main():
 CACHE.mkdir(exist_ok=True);rows=[];sources=set()
 with closing(sqlite3.connect(ROOT/'build/terrain-shared-allocation-error-detail20-direct-e800.sqlite3')) as mesh:manifest=json.loads(mesh.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
 with closing(sqlite3.connect(OUTPUT)) as db,db:
  db.executescript('CREATE TABLE IF NOT EXISTS images(region TEXT,size INTEGER,data BLOB,PRIMARY KEY(region,size));CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);')
  for area in manifest['reliefRegions']:
   classes,used,origin=classes_for_area(area);sources.update(used);image=Image.fromarray(colorize(classes));counts=dict(zip(*[v.tolist() for v in np.unique(classes,return_counts=True)]));print('分類を取得',area['id'],counts,flush=True)
   for size in SIZES:
    resized=image if size==4096 else image.resize((size,size),Image.Resampling.BOX)
    # 陰影なしの色だけをパレットPNGへ量子化し、転送容量を小さくする。
    encoded=resized.quantize(colors=256,method=Image.Quantize.MEDIANCUT,dither=Image.Dither.NONE);stream=io.BytesIO();encoded.save(stream,format='PNG',compress_level=9);body=stream.getvalue()
    db.execute('INSERT OR REPLACE INTO images VALUES(?,?,?)',(area['id'],size,body));row=dict(region=area['id'],size=size,bytes=len(body),origin=origin,classes=counts);rows.append(row);print('地表色を保存',area['id'],size,len(body),flush=True)
  meta=dict(version=1,source='ESA WorldCover 2021 v200',sourceURL='https://github.com/ESA-WorldCover/esa-worldcover-datasets',license='CC BY 4.0',attribution=ATTRIBUTION,shadeIncluded=False,widthMeters=128000,sizes=SIZES,format='palette-png',sourceOverviewMeters=20,palette=PALETTE,sources=sorted(sources),images=rows)
  db.execute('INSERT OR REPLACE INTO metadata VALUES("manifest",?)',(json.dumps(meta),))
 (ROOT/'build/terrain-landcover-color.json').write_text(json.dumps(meta,ensure_ascii=False,indent=2),encoding='utf-8')
def add_small_sizes():
 with closing(sqlite3.connect(OUTPUT)) as db,db:
  meta=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
  for region,body in db.execute('SELECT region,data FROM images WHERE size=4096').fetchall():
   image=Image.open(io.BytesIO(body)).convert('RGB')
   for size in [128,256]:
    encoded=image.resize((size,size),Image.Resampling.BOX).quantize(colors=256,method=Image.Quantize.MEDIANCUT,dither=Image.Dither.NONE)
    stream=io.BytesIO();encoded.save(stream,format='PNG',compress_level=9);data=stream.getvalue()
    db.execute('INSERT OR REPLACE INTO images VALUES(?,?,?)',(region,size,data))
    meta['images']=[r for r in meta['images'] if (r['region'],r['size'])!=(region,size)]
    reference=next(r for r in meta['images'] if r['region']==region and r['size']==4096)
    meta['images'].append(dict(reference,size=size,bytes=len(data),resizedFrom=4096))
    print(region,size,len(data),flush=True)
  meta['sizes']=SIZES;db.execute('UPDATE metadata SET value=? WHERE key="manifest"',(json.dumps(meta),))
 (ROOT/'build/terrain-landcover-color.json').write_text(json.dumps(meta,ensure_ascii=False,indent=2),encoding='utf-8')

if __name__=='__main__':
 import argparse
 parser=argparse.ArgumentParser();parser.add_argument('--add-small',action='store_true');args=parser.parse_args()
 if args.add_small:add_small_sizes()
 else:main()
