"""同じ地域の L14 標高から、全 PNG レベルを直接生成する。"""
import json
from pathlib import Path
import shutil

import numpy as np
from PIL import Image
from scipy.ndimage import zoom


def resize(values,width,height):
    return np.asarray(Image.fromarray(values.astype(np.float32)).resize((width,height),Image.Resampling.BOX))


def build(root):
    meta=json.loads((root/'manifest.json').read_text(encoding='utf-8'))
    with Image.open(root/'height.png') as image:
        pixels=np.asarray(image)
    valid=pixels[:,:,2]>0
    heights=pixels[:,:,0].astype(np.float32)+256*pixels[:,:,1].astype(np.float32)+meta['heightOffset']
    h,w=heights.shape;meters=meta['meters']
    safe=np.where(valid,heights,0)
    padded=np.pad(np.where(valid,heights,np.nan),1,mode='edge')
    left,right=padded[1:-1,:-2],padded[1:-1,2:]
    up,down=padded[:-2,1:-1],padded[2:,1:-1]
    dx=(np.where(np.isfinite(right),right,heights)-np.where(np.isfinite(left),left,heights))/(2*meters)
    dy=(np.where(np.isfinite(down),down,heights)-np.where(np.isfinite(up),up,heights))/(2*meters)
    length=np.sqrt(1+dx*dx+dy*dy)
    nx=np.where(valid,-dx/length,0);ny=np.where(valid,dy/length,0)
    # 全標高レベルで共用し、各解像度を元の L14 の法線から直接作る。
    normal_folder=root/'normals';normal_folder.mkdir(exist_ok=True)
    normal_variants=[]
    size=min(2048,w//2,h//2)
    while size>=4:
        normal=np.empty((size,size,3),np.uint8)
        normal[:,:,:2]=np.rint((np.stack([resize(nx,size,size),resize(ny,size,size)],axis=-1)+1)*127.5).clip(0,255).astype(np.uint8)
        normal[:,:,2]=255
        path=normal_folder/f'{size}.png';Image.fromarray(normal).save(path,optimize=True)
        normal_variants.append(dict(size=size,bytes=path.stat().st_size,url=f'/normals/{size}.png'))
        size//=2
    levels=[]
    for level in range(14,3,-1):
        folder=root/'levels'/str(level);folder.mkdir(parents=True,exist_ok=True)
        factor=2**(14-level);width=max(2,w//factor);height=max(2,h//factor)
        if level==14:
            for name in ['height.png','normal.png','normal-coarse.png']:
                shutil.copyfile(root/name,folder/name)
            item=dict(meta)
            item.update(level=level,normalCoarseWidth=w//4,normalCoarseHeight=h//4,maxReconstructionErrorMeters=0)
        else:
            weights=resize(valid,width,height)
            z=resize(safe,width,height)/np.maximum(weights,1e-10)
            mask=weights>0
            q=np.rint(z-meta['heightOffset']).clip(0,65535).astype(np.uint16)
            packed=np.stack([q&255,q>>8,mask.astype(np.uint16)*255],axis=-1).astype(np.uint8)
            Image.fromarray(packed).save(folder/'height.png',optimize=True)
            sizes=[(max(2,min(w//2,max(256,width//2))),max(2,min(h//2,max(256,height//2)))),
                   (max(2,min(w//4,max(128,width//4))),max(2,min(h//4,max(128,height//4))))]
            for name,(nw,nh) in zip(['normal.png','normal-coarse.png'],sizes):
                normal=np.empty((nh,nw,3),np.uint8)
                normal[:,:,:2]=np.rint((np.stack([resize(nx,nw,nh),resize(ny,nw,nh)],axis=-1)+1)*127.5).clip(0,255).astype(np.uint8)
                normal[:,:,2]=255;Image.fromarray(normal).save(folder/name,optimize=True)
            restored=zoom(q.astype(np.float32)+meta['heightOffset'],(h/height,w/width),order=1,prefilter=False)
            error=float(np.abs(restored[valid]-heights[valid]).max())
            item=dict(meta,level=level,width=width,height=height,meters=meters*(w-1)/(width-1),
                      normalWidth=sizes[0][0],normalHeight=sizes[0][1],normalCoarseWidth=sizes[1][0],normalCoarseHeight=sizes[1][1],
                      pngMaxHeightErrorMeters=float(np.abs(q[mask].astype(np.float32)+meta['heightOffset']-z[mask]).max()),
                      maxReconstructionErrorMeters=error)
        item.update(extentMetersX=(w-1)*meters,extentMetersY=(h-1)*meters,
                    heightBytes=(folder/'height.png').stat().st_size,normalBytes=(folder/'normal.png').stat().st_size,
                    coarseNormalBytes=(folder/'normal-coarse.png').stat().st_size,
                    sourceLevel=14,heightFilter='各レベルを L14 から直接面積平均',normalFilter='L14 の面の向きを求めてから面積平均')
        (folder/'manifest.json').write_text(json.dumps(item,ensure_ascii=False,indent=2),encoding='utf-8')
        levels.append(item)
        print(json.dumps(dict(level=level,width=width,heightBytes=item['heightBytes'],normalBytes=item['coarseNormalBytes'])),flush=True)
    report=dict(levels=levels,normalVariants=normal_variants,defaultLevel=14,totalStandardBytes=sum(r['heightBytes']+r['coarseNormalBytes'] for r in levels),
                totalSharedNormalBytes=sum(r['bytes'] for r in normal_variants),
                totalAllPNGBytes=sum(r['heightBytes']+r['coarseNormalBytes']+r['normalBytes'] for r in levels))
    (root/'levels.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
    return report


if __name__=='__main__':
    import argparse
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--root',type=Path,default=Path('build/terrain-heightmap-yari'))
    build(p.parse_args().root)
