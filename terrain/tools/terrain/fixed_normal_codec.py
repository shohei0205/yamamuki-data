"""メッシュ比較専用の法線画像圧縮。配信経路とは分けて実行する。"""
from io import BytesIO
import hashlib
import json
import threading
import time
import subprocess
from pathlib import Path
import numpy as np
from PIL import Image
from dynamic_jxl import convert, cli

ROOT = Path(__file__).resolve().parents[2]
LOCK = threading.Lock()

def compare(body, codec, quality, distance, mode, block=6, correction=False):
    if codec not in ('png','jpeg','webp','jxl','astc') or not 1 <= quality <= 100 or not 0 <= distance <= 5 or mode not in ('auto','modular'):
        raise ValueError('圧縮設定が不正です')
    if block not in (4,5,6,8,10,12):raise ValueError('ASTCブロックサイズが不正です')
    key=hashlib.sha256(body+f'{codec}/{quality}/{distance}/{mode}/{block}/{correction}/4'.encode()).hexdigest()
    folder=ROOT/'build/fixed-normal-codec'/key
    with LOCK:
        if (folder/'report.json').exists():
            return (folder/'preview.png').read_bytes(),json.loads((folder/'report.json').read_text())
        folder.mkdir(parents=True,exist_ok=True)
        source=Image.open(BytesIO(body));gray=source.mode=='L';image=source.convert('L' if gray else 'RGB')
        started=time.perf_counter()
        encoded=folder/('encoded.'+codec)
        if codec=='astc':
            tool=ROOT/'build/terrain-codec-tools/astc/bin/astcenc-avx2.exe'
            inp=folder/'source.png';image.save(inp)
            def run(args):
                subprocess.run([str(tool),*map(str,args)],check=True,capture_output=True,timeout=120,creationflags=subprocess.CREATE_NO_WINDOW)
            run(['-cl',inp,encoded,f'{block}x{block}',str(quality),*(['-esw','rrr1','-ssw','r'] if gray else [] if correction else ['-normal']),'-j','4','-silent'])
            enc_ms=(time.perf_counter()-started)*1000
            started=time.perf_counter()
            run(['-dl',encoded,folder/'preview.png','-dsw','rrr1' if gray else 'rgba' if correction else 'raz1','-j','4','-silent'])
            dec_ms=(time.perf_counter()-started)*1000
            decoded=Image.open(folder/'preview.png').convert('L' if gray else 'RGB')
        elif codec=='jxl':
            inp=folder/'source.png';image.save(inp)
            if gray:
                started=time.perf_counter()
                cli('cjxl.exe',[inp,encoded,'-d',distance,'-e',7,'--num_threads=4','-x','color_space=Gra_D65_Rel_Lin','--quiet',*(['--modular=1'] if mode=='modular' else [])])
                enc_ms=(time.perf_counter()-started)*1000;started=time.perf_counter()
                cli('djxl.exe',[encoded,folder/'preview.png','--bits_per_sample=8','--color_space=Gra_D65_Rel_Lin','--num_threads=4','--quiet'])
                dec_ms=(time.perf_counter()-started)*1000
            else:enc_ms,dec_ms=convert(inp,encoded,folder/'preview.png',distance,False,mode)
            decoded=Image.open(folder/'preview.png').convert('L' if gray else 'RGB')
        else:
            if codec=='png':image.save(encoded,format='PNG',optimize=True)
            elif codec=='jpeg':image.save(encoded,format='JPEG',quality=quality,subsampling=0)
            else:image.save(encoded,format='WEBP',quality=quality,method=6)
            enc_ms=(time.perf_counter()-started)*1000
            started=time.perf_counter();decoded=Image.open(encoded).convert('L' if gray else 'RGB');decoded.load()
            dec_ms=(time.perf_counter()-started)*1000
        def vectors(im):
            v=np.asarray(im,dtype=np.float64)/127.5-1
            return v/np.maximum(np.linalg.norm(v,axis=2,keepdims=True),1e-12)
        angles=np.zeros((1,1)) if gray else np.degrees(np.arccos(np.clip((vectors(image)*vectors(decoded)).sum(axis=2),-1,1)))
        decoded.save(folder/'preview.png')
        report=dict(grayscale=gray,storage=('グレースケール1成分' if gray and codec in ('png','jpeg','jxl') else 'R成分のみ使用' if gray and codec=='astc' else 'WebP RGB形式' if gray else 'RGB'),correction=correction,rgbRMSE=float(np.sqrt(np.mean((np.asarray(image,dtype=np.float64)-np.asarray(decoded,dtype=np.float64))**2))),codec=codec,bytes=encoded.stat().st_size,sourceBytes=len(body),encodeMs=enc_ms,decodeMs=dec_ms,meanAngle=float(angles.mean()),p95Angle=float(np.percentile(angles,95)),maxAngle=float(angles.max()),width=image.width,height=image.height)
        if codec=='astc':report.update(block=block,bitsPerPixel=128/(block*block),gpuBytes=encoded.stat().st_size-16)
        (folder/'report.json').write_text(json.dumps(report),encoding='utf-8')
        return (folder/'preview.png').read_bytes(),report


def mesh_normal_grid(mesh, extent, resolution):
    """表示メッシュの頂点法線を画像解像度へ平均する。"""
    import struct,zlib
    from scipy.interpolate import LinearNDInterpolator
    raw=zlib.decompress(mesh);nv,ni=struct.unpack_from('<II',raw)
    vertices=np.frombuffer(raw,'<f4',nv*3,8).reshape(-1,3)
    faces=np.frombuffer(raw,'<u4',ni,8+nv*12).reshape(-1,3)
    normals=np.zeros(vertices.shape,np.float64)
    tri=vertices[faces].astype(np.float64)
    directions=np.cross(tri[:,1]-tri[:,0],tri[:,2]-tri[:,0]);directions[directions[:,2]<0]*=-1
    for axis in range(3):np.add.at(normals,faces[:,axis],directions)
    normals/=np.maximum(np.linalg.norm(normals,axis=1,keepdims=True),1e-12)
    interpolator=LinearNDInterpolator(vertices[:,:2],normals)
    factor=max(1,min(4,1024//resolution));size=resolution*factor
    positions=(np.arange(size)+.5)/size*extent-extent/2
    baseline=np.empty((resolution,resolution,3),np.float64)
    for row in range(resolution):
        yy=np.repeat(-positions[row*factor:(row+1)*factor],size)
        xx=np.tile(positions,factor)
        samples=interpolator(xx,yy).reshape(factor,resolution,factor,3)
        samples/=np.maximum(np.linalg.norm(samples,axis=3,keepdims=True),1e-12)
        baseline[row]=samples.mean(axis=(0,2))
    baseline/=np.maximum(np.linalg.norm(baseline,axis=2,keepdims=True),1e-12)
    return baseline


def correction_png(body, mesh, extent, resolution, columns=1, rows=1):
    from fixed_mesh_groups import atlas_png
    baseline=mesh_normal_grid(mesh,extent,resolution)
    original=Image.open(BytesIO(body)).convert('RGB')
    # 階層画像の各セルは同じ地域の独立コピー。
    detail=np.asarray(original,dtype=np.float64)
    if columns!=1 or rows!=1:
        detail=detail[2:2+resolution,2:2+resolution]
    detail=detail/127.5-1
    detail/=np.maximum(np.linalg.norm(detail,axis=2,keepdims=True),1e-12)
    delta=detail-baseline
    output=BytesIO();Image.fromarray(np.rint(np.clip(128+delta*63.5,0,255)).astype('uint8')).save(output,format='PNG')
    return atlas_png(output.getvalue(),columns,rows) if columns!=1 or rows!=1 else output.getvalue()


def baked_texture(normal_body, height_body, grid_size, resolution, mode, azimuth=315, altitude=40, columns=1, rows=1):
    """固定陰影、または複数の局所平均で谷を暗くする模様を作る。"""
    import zlib
    from scipy.ndimage import gaussian_filter
    from fixed_mesh_groups import atlas_png
    if mode=='baked':
        normals=np.asarray(Image.open(BytesIO(normal_body)).convert('RGB'),dtype=np.float64)/127.5-1
        normals/=np.maximum(np.linalg.norm(normals,axis=2,keepdims=True),1e-12)
        az,alt=np.radians([azimuth,altitude]);sun=np.array([np.sin(az)*np.cos(alt),np.cos(az)*np.cos(alt),np.sin(alt)])
        values=.3+.7*np.maximum(normals@sun,0)
    elif mode=='valley':
        heights=np.frombuffer(zlib.decompress(height_body),'<f4').reshape(grid_size,grid_size)
        valley=sum(np.maximum(gaussian_filter(heights,sigma)-heights,0)/scale for sigma,scale in [(2,20),(8,60),(24,140)])/3
        values=np.exp(-.7*valley).clip(.45,1)
    else:raise ValueError('焼付方式が不正です')
    image=Image.fromarray(np.rint(np.clip(values,0,1)*255).astype('uint8')).resize((resolution,resolution),Image.Resampling.BOX)
    out=BytesIO();image.save(out,format='PNG',optimize=True)
    return atlas_png(out.getvalue(),columns,rows) if columns!=1 or rows!=1 else out.getvalue()


def baked_mesh_correction(body, mesh, extent, resolution, azimuth, altitude, columns=1, rows=1):
    """焼付陰影とメッシュ陰影の比を対数で記録し、細かな形状を残す。"""
    from fixed_mesh_groups import atlas_png
    original=np.asarray(Image.open(BytesIO(body)).convert('L'),dtype=np.float64)/255
    if columns!=1 or rows!=1:original=original[2:2+resolution,2:2+resolution]
    baseline=mesh_normal_grid(mesh,extent,resolution)
    az,alt=np.radians([azimuth,altitude]);sun=np.array([np.sin(az)*np.cos(alt),np.cos(az)*np.cos(alt),np.sin(alt)])
    light=.3+.7*np.maximum(baseline@sun,0)
    ratio=np.maximum(original,1/255)/light
    values=np.rint(np.clip(128+63.5*np.log2(ratio),0,255)).astype('uint8')
    output=BytesIO();Image.fromarray(values).save(output,format='PNG',optimize=True)
    return atlas_png(output.getvalue(),columns,rows) if columns!=1 or rows!=1 else output.getvalue()
