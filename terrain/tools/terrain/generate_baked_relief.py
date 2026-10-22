"""比較用DEMから、メッシュ品質に依存しないグレースケール陰影を保存する。"""
import io,json,sqlite3
from contextlib import closing
import numpy as np
from PIL import Image
from scipy.ndimage import gaussian_filter,map_coordinates,distance_transform_edt
from national_independent_mesh import ROOT
OUTPUT=ROOT/'build/terrain-baked-relief.sqlite3'
SIZES=[512,1024,2048,4096]
def hillshade(grid,spacing=31.25):
    """西～北からの4方向を合成し、尾根・谷の向きによる見落としを減らす。"""
    gy,gx=np.gradient(np.asarray(grid,np.float32),spacing)
    length=np.sqrt(1+gx*gx+gy*gy);aspect=np.arctan2(-gx,-gy)
    height=np.deg2rad(35);weighted=np.zeros_like(gx);weights=np.zeros_like(gx)
    for azimuth in [225,270,315,360]:
        az=np.deg2rad(azimuth);weight=np.sin(aspect-az)**2
        lit=np.maximum(0,(-gx*np.sin(az)*np.cos(height)-gy*np.cos(az)*np.cos(height)+np.sin(height))/length)
        weighted+=weight*lit;weights+=weight
    # 平坦面は約180/255。色は含めず、表示時に自由に設定できる。
    shade=np.clip((.28+.72*weighted/np.maximum(weights,1e-8)),0,1)
    return np.rint(shade*255).astype(np.uint8)
def image_for_size(grid,size,width=128000.):
    step=(grid.shape[0]-1)/(size-1)
    filtered=gaussian_filter(grid,sigma=max(.0,.45*step),mode='nearest') if step>1.2 else grid
    coords=np.linspace(0,grid.shape[0]-1,size,dtype=np.float32);yy,xx=np.meshgrid(coords,coords,indexing='ij')
    height=map_coordinates(filtered,[yy,xx],order=1,mode='nearest')
    return hillshade(height,width/(size-1))
def main():
    rows=[]
    with closing(sqlite3.connect(OUTPUT)) as db,db:
        db.executescript('CREATE TABLE IF NOT EXISTS images(region TEXT,size INTEGER,data BLOB,PRIMARY KEY(region,size));CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);')
        for name in ['fuji','tsukuba','yari']:
            with np.load(ROOT/f'build/terrain-quality-source-cache-v1/{name}.npz') as source:grid=source['grid'].astype(np.float32);valid=source['valid']
            if not valid.all():
                nearest=distance_transform_edt(~valid,return_distances=False,return_indices=True);grid=grid[tuple(nearest)];del nearest
            for size in SIZES:
                pixels=image_for_size(grid,size);stream=io.BytesIO();Image.fromarray(pixels).save(stream,format='PNG',compress_level=9);body=stream.getvalue()
                assert Image.open(io.BytesIO(body)).mode=='L'
                db.execute('INSERT OR REPLACE INTO images VALUES(?,?,?)',(name,size,body));row=dict(region=name,size=size,bytes=len(body),spacing=128000/(size-1));rows.append(row);print(row,flush=True)
            del grid,valid
        db.execute('INSERT OR REPLACE INTO metadata VALUES(?,?)',('manifest',json.dumps(dict(version=1,widthMeters=128000,sizes=SIZES,format='grayscale8-png',lightAzimuths=[225,270,315,360],lightAltitude=35,images=rows))))
    (ROOT/'build/terrain-baked-relief.json').write_text(json.dumps(rows,ensure_ascii=False,indent=2),encoding='utf-8')
if __name__=='__main__':main()
