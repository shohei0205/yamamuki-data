"""平野の地表色を平滑化し、水域と山地の色を残す。"""
import io,json,sqlite3
import numpy as np
from PIL import Image
from scipy.ndimage import gaussian_filter,distance_transform_edt
from national_independent_mesh import ROOT

def hill_mask(grid,spacing=31.25):
    height=gaussian_filter(np.asarray(grid,np.float32),125/spacing)
    gy,gx=np.gradient(height,spacing)
    slope=np.degrees(np.arctan(np.hypot(gx,gy)))
    weight=np.clip((slope-2)/6,0,1)
    weight=weight*weight*(3-2*weight)
    return gaussian_filter(weight,62.5/spacing)

def denoise_plains(rgb,hills,spacing=128000/4095):
    # 水域をぼかしに混ぜず、元の川・湖の画素はそのまま残す。
    water=(rgb[:,:,2]>rgb[:,:,0]+10)&(rgb[:,:,2]>rgb[:,:,1]+8)
    land=(~water).astype(np.float32)
    weight=gaussian_filter(land,250/spacing)
    smooth=np.stack([gaussian_filter(rgb[:,:,c]*land,250/spacing)/np.maximum(weight,1e-6) for c in range(3)],axis=-1)
    blend=(1-hills)*.9
    blend[water]=0
    return rgb*(1-blend[:,:,None])+smooth*blend[:,:,None]

def main():
    source=sqlite3.connect(ROOT/'build/terrain-landcover-color.sqlite3')
    target=ROOT/'build/terrain-landcover-color-hills.sqlite3'
    with sqlite3.connect(target) as db:
        db.executescript('CREATE TABLE IF NOT EXISTS images(region TEXT,size INTEGER,data BLOB,PRIMARY KEY(region,size));CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);')
        meta=json.loads(source.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
        for region in ['fuji','tsukuba','yari']:
            with np.load(ROOT/f'build/terrain-quality-source-cache-v1/{region}.npz') as data:
                grid=data['grid'].astype(np.float32);valid=data['valid']
            if not valid.all():
                nearest=distance_transform_edt(~valid,return_distances=False,return_indices=True)
                grid=grid[tuple(nearest)]
            mask=Image.fromarray(np.rint(hill_mask(grid)*255).astype(np.uint8))
            original=Image.open(io.BytesIO(source.execute('SELECT data FROM images WHERE region=? AND size=4096',(region,)).fetchone()[0])).convert('RGB')
            rgb=np.asarray(original,dtype=np.float32)
            filtered=denoise_plains(rgb,np.asarray(mask.resize(original.size),dtype=np.float32)/255)
            for size,body in source.execute('SELECT size,data FROM images WHERE region=?',(region,)):
                image=Image.fromarray(np.rint(filtered).clip(0,255).astype(np.uint8)).resize((size,size),Image.Resampling.BOX).convert('RGBA')
                # アルファは透明度ではなく、追加模様を出す山地の重みとして使う。
                image.putalpha(Image.fromarray((128+np.asarray(mask.resize((size,size),Image.Resampling.BOX),dtype=np.uint16)*127//255).astype(np.uint8)))
                output=io.BytesIO();image.save(output,format='PNG',compress_level=9)
                db.execute('INSERT OR REPLACE INTO images VALUES(?,?,?)',(region,size,output.getvalue()))
                print(region,size,len(output.getvalue()),flush=True)
        meta.update(format='rgba-png',plainSuppression=False,plainDenoiseMeters=250,alphaUsage='procedural-weight',slopeTransitionDegrees=[2,8],demSmoothingMeters=125)
        db.execute('INSERT OR REPLACE INTO metadata VALUES("manifest",?)',(json.dumps(meta),))
    source.close()

if __name__=='__main__':main()
