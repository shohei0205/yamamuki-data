"""比較用TINの全品質で共通に使うDEM法線PNGを生成する。"""
import json,sqlite3
import numpy as np
from PIL import Image
from scipy.ndimage import gaussian_filter,map_coordinates,convolve1d
from scipy.signal import firwin
from progressive_tin_compare import ROOT,REGIONS
from fixed_mesh_preview import source_grid

def sinc_lpf(values,ratio):
    if ratio<=1:return values.copy()
    # 出力のナイキスト周波数を遮断周波数にしたKaiser窓付きFIR。
    radius=int(np.ceil(4*ratio))
    kernel=firwin(2*radius+1,1/ratio,window=('kaiser',8.6),scale=True)
    return convolve1d(convolve1d(values,kernel,axis=0,mode='nearest'),kernel,axis=1,mode='nearest')

if __name__ == '__main__':
    folder=ROOT/'build/progressive-normal-maps'
    folder.mkdir(exist_ok=True)
    report={}
    filtered_report={}
    lpf_report={}
    with sqlite3.connect((ROOT/'build/terrain-lod.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as db:
        for region,(_,lat,lon) in REGIONS.items():
            grid,_=source_grid(db,lat,lon,extent=16000,size=685)
            dy,dx=np.gradient(gaussian_filter(grid.astype(float),1),16000/684)
            report[region]={}
            filtered_report[region]={}
            lpf_report[region]={}
            base=np.stack([-dx,dy,np.ones_like(dx)],axis=-1)
            base/=np.linalg.norm(base,axis=-1,keepdims=True)
            for size in [128,256,512,685]:
                y,x=np.meshgrid(np.linspace(0,684,size),np.linspace(0,684,size),indexing='ij')
                normals=np.stack([-map_coordinates(dx,[y,x],order=1),map_coordinates(dy,[y,x],order=1),np.ones_like(x)],axis=-1)
                normals/=np.linalg.norm(normals,axis=-1,keepdims=True)
                pixels=np.rint(np.clip(normals*.5+.5,0,1)*255).astype(np.uint8)
                path=folder/f'{region}-{size}.png'
                Image.fromarray(pixels).save(path,optimize=True)
                report[region][size]=path.stat().st_size
                ratio=684/(size-1);sigma=.5*np.sqrt(max(0,ratio*ratio-1))
                averaged=gaussian_filter(base,(sigma,sigma,0),mode='nearest') if sigma>0 else base
                normal=np.stack([map_coordinates(averaged[:,:,c],[y,x],order=1) for c in range(3)],axis=-1)
                normal/=np.maximum(np.linalg.norm(normal,axis=-1,keepdims=True),1e-12)
                image=np.rint(np.clip(normal*.5+.5,0,1)*255).astype(np.uint8)
                filtered=folder/f'{region}-{size}-filtered.png'
                Image.fromarray(image).save(filtered,optimize=True)
                filtered_report[region][size]=filtered.stat().st_size
                low=sinc_lpf(base,ratio)
                normal=np.stack([map_coordinates(low[:,:,c],[y,x],order=1) for c in range(3)],axis=-1)
                normal[:,:,2]=np.maximum(normal[:,:,2],1e-6)
                normal/=np.maximum(np.linalg.norm(normal,axis=-1,keepdims=True),1e-12)
                path=folder/f'{region}-{size}-lpf.png'
                Image.fromarray(np.rint(np.clip(normal*.5+.5,0,1)*255).astype(np.uint8)).save(path,optimize=True)
                lpf_report[region][size]=path.stat().st_size
    (folder/'manifest.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
    (folder/'filtered-manifest.json').write_text(json.dumps(filtered_report,indent=2),encoding='utf-8')
    (folder/'lpf-manifest.json').write_text(json.dumps(lpf_report,indent=2),encoding='utf-8')
    print(json.dumps(lpf_report))
