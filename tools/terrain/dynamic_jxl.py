"""比較画面向けに JPEG XL をタイル単位で符号化・復号する。全国生成とは独立。"""
import json
import math
from pathlib import Path
import subprocess
import struct
import threading
import time
import uuid

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
BIN = ROOT/'build/terrain-codec-tools/jxl-0.12.0/x64-windows-static/bin'
OUT = ROOT/'build/terrain-jxl-dynamic'
LOCK = threading.Lock()


def validate(options):
    level = options.get('level', 11)
    size = options.get('normalSize', 512)
    if type(level) is not int or level not in range(4, 15):
        raise ValueError('標高レベルは 4〜14 を指定してください')
    if type(size) is not int or size not in [2**i for i in range(2, 12)]:
        raise ValueError('法線サイズが不正です')
    distances = []
    for name in ['heightDistance', 'normalDistance']:
        value = options.get(name, 0 if name == 'heightDistance' else .3)
        if type(value) not in (int, float) or not math.isfinite(value) or not 0 <= value <= 5:
            raise ValueError('圧縮設定 d は 0〜5 を指定してください')
        distances.append(float(value))
    return level, size, *distances


def cli(name, args):
    subprocess.run([str(BIN/name), *map(str, args)], check=True, capture_output=True,
                   timeout=120, creationflags=subprocess.CREATE_NO_WINDOW)


def convert(input_path, jxl, decoded, distance, gray, mode='auto'):
    color = 'Gra_D65_Rel_Lin' if gray else 'RGB_D65_SRG_Rel_Lin'
    started = time.perf_counter()
    args=[input_path, jxl, '-d', distance, '-e', 7, '--num_threads=4', '-x', f'color_space={color}', '--quiet']
    if mode == 'modular':
        args += ['--modular=1']
    cli('cjxl.exe', args)
    encoded_ms = (time.perf_counter()-started)*1000
    started = time.perf_counter()
    cli('djxl.exe', [jxl, decoded, '--bits_per_sample=16' if gray else '--bits_per_sample=8',
                     f'--color_space={color}', '--num_threads=4', '--quiet'])
    return encoded_ms, (time.perf_counter()-started)*1000


def direction(pixels):
    xy = pixels[..., :2].astype(np.float64)/127.5-1
    xyz = np.concatenate([xy, np.sqrt(np.maximum(0, 1-(xy*xy).sum(axis=-1)))[..., None]], axis=-1)
    return xyz/np.maximum(1e-12, np.linalg.norm(xyz, axis=-1, keepdims=True))


def encode(options):
    level, size, hd, nd = validate(options)
    tile_grid=options.get('tileGrid',0)
    if type(tile_grid) is not int or tile_grid not in (0,2):
        raise ValueError('実タイル表示の指定が不正です')
    seams=options.get('seams','legacy')
    if seams not in ('legacy','overlap'):
        raise ValueError('境界対策が不正です')
    modes=[options.get(key,'auto') for key in ['heightMode','normalMode']]
    if any(mode not in ('auto','modular') for mode in modes):
        raise ValueError('符号化方式は auto または modular を指定してください')
    region = options.get('region', 'yari')
    if region not in ('yari', 'tsukuba'):
        raise ValueError('地域が不正です')
    if not LOCK.acquire(blocking=False):
        raise BlockingIOError('別の比較を処理中です。完了後に再実行してください')
    try:
        return process(level, size, hd, nd, region, *modes, seams, tile_grid)
    finally:
        LOCK.release()


def blend_tile(accum, weights, tile, x, y, core_width, core_height, padding, valid=None):
    """隣接タイルの余白を重ね、外側に向かって重みを減らす。"""
    height, width = weights.shape
    x0, y0 = max(0, x-padding), max(0, y-padding)
    x1, y1 = min(width, x+core_width+padding), min(height, y+core_height+padding)
    sx, sy = x0-x+padding, y0-y+padding
    def ramp(length):
        i=np.arange(length,dtype=np.float32)
        return np.minimum(1,np.minimum(i/padding,(length-1-i)/padding))
    weight=ramp(tile.shape[0])[sy:sy+y1-y0,None]*ramp(tile.shape[1])[None,sx:sx+x1-x0]
    if valid is not None:weight=weight*valid[y0:y1,x0:x1]
    region=tile[sy:sy+y1-y0,sx:sx+x1-x0]
    accum[y0:y1,x0:x1]+=region*(weight[...,None] if tile.ndim==3 else weight)
    weights[y0:y1,x0:x1]+=weight


def process(level, size, hd, nd, region='yari', height_mode='auto', normal_mode='auto', seams='legacy', tile_grid=0):
    folder = OUT/uuid.uuid4().hex
    folder.mkdir(parents=True)
    source = ROOT/(f'build/terrain-tile-preview-{region}' if tile_grid else f'build/terrain-heightmap-{region}')/'levels'/str(level)
    normal_path=(source/'normals'/f'{size}.png') if tile_grid else ROOT/f'build/terrain-heightmap-{region}/normals/{size}.png'
    if not normal_path.is_file():raise ValueError('このレベルの法線解像度は未生成です')
    meta = json.loads((source/'manifest.json').read_text(encoding='utf-8'))
    p = np.asarray(Image.open(source/'height.png').convert('RGB'))
    valid = p[..., 2] == 255
    heights = p[..., 0].astype(np.float32)+256*p[..., 1].astype(np.float32)+meta['heightOffset']
    restored = np.full(heights.shape, np.nan, np.float32)
    padding=16 if seams=='overlap' and max(heights.shape)>256 else 0
    padded=np.pad(heights,padding,mode='edge');padded_valid=np.pad(valid,padding,mode='edge')
    height_accum=np.zeros_like(heights);height_weights=np.zeros_like(heights)
    shared_min=float(heights[valid].min()) if valid.any() else 0
    shared_span=float(heights[valid].max()-shared_min) if valid.any() else 0
    timings = {'encodeMs': 0., 'decodeMs': 0.}
    height_bytes = 0
    height_tiles = []
    # 全国版と同じ 256 四方で処理し、地域一括による過大な圧縮率を避ける。
    for y in range(0, heights.shape[0], 256):
        for x in range(0, heights.shape[1], 256):
            ch,cw=heights[y:y+256,x:x+256].shape
            tile = padded[y:y+ch+padding*2,x:x+cw+padding*2]
            mask = padded_valid[y:y+ch+padding*2,x:x+cw+padding*2]
            minimum = float(tile[mask].min()) if mask.any() else 0
            span = float(tile[mask].max()-minimum) if mask.any() else 0
            if seams=='overlap':minimum,span=shared_min,shared_span
            scale = 1. if hd == 0 else max(span, 1.)/65535
            q = np.rint(np.where(mask, (tile-minimum)/scale, 0)).clip(0, 65535).astype(np.uint16)
            stem = folder/f'h-{x}-{y}'
            inp, encoded, decoded = stem.with_suffix('.pgm'), stem.with_suffix('.jxl'), stem.with_suffix('.png')
            inp.write_bytes(f'P5\n{tile.shape[1]} {tile.shape[0]}\n65535\n'.encode()+q.astype('>u2').tobytes())
            a, b = convert(inp, encoded, decoded, hd, True, height_mode)
            timings['encodeMs'] += a; timings['decodeMs'] += b
            out = np.asarray(Image.open(decoded)).astype(np.uint16)
            if hd == 0 and not np.array_equal(q, out):
                raise ValueError('標高の可逆照合に失敗しました')
            if padding:blend_tile(height_accum,height_weights,minimum+out.astype(np.float32)*scale,x,y,cw,ch,padding,valid)
            core=out[padding:padding+ch,padding:padding+cw]
            restored[y:y+ch, x:x+cw] = np.where(valid[y:y+ch,x:x+cw], minimum+core.astype(np.float32)*scale, np.nan)
            height_bytes += encoded.stat().st_size
            height_tiles.append((x, y, minimum, scale, encoded.read_bytes()))
            inp.unlink(); decoded.unlink()
    if padding:restored=np.divide(height_accum,height_weights,out=np.full_like(heights,np.nan),where=height_weights>0)
    # 比較値は画像本体だけでなく、タイル管理値と欠損マスクも含める。
    import zlib
    mask_body = zlib.compress(np.packbits(valid, bitorder='little').tobytes(), 9)
    mask_bytes = len(mask_body)
    tile_count = math.ceil(heights.shape[0]/256)*math.ceil(heights.shape[1]/256)
    with (folder/'height.yjd').open('wb') as stream:
        stream.write((b'YJD2'+struct.pack('<HHIII',heights.shape[1],heights.shape[0],tile_count,mask_bytes,padding)) if seams=='overlap' else b'YJD1'+struct.pack('<HHII', heights.shape[1], heights.shape[0], tile_count, mask_bytes))
        for x, y, minimum, scale, body in height_tiles:
            stream.write(struct.pack('<HHffI', x, y, minimum, scale, len(body)))
            stream.write(body)
        stream.write(mask_body)
    height_error = np.abs(restored[valid]-heights[valid])
    restored.astype('<f4').tofile(folder/'height.f32')
    original = np.asarray(Image.open(normal_path).convert('RGB'))
    normal = np.empty_like(original)
    normal_padding=16 if seams=='overlap' and size>256 else 0
    padded_normal=np.pad(original,((normal_padding,normal_padding),(normal_padding,normal_padding),(0,0)),mode='edge')
    normal_accum=np.zeros(original.shape,np.float32);normal_weights=np.zeros(original.shape[:2],np.float32)
    normal_bytes = 0
    normal_tiles = []
    for y in range(0, size, 256):
        for x in range(0, size, 256):
            ch,cw=original[y:y+256,x:x+256].shape[:2]
            tile = padded_normal[y:y+ch+normal_padding*2,x:x+cw+normal_padding*2]
            stem = folder/f'n-{x}-{y}'
            inp, encoded, decoded = stem.with_suffix('.png'), stem.with_suffix('.jxl'), stem.with_suffix('.decoded.png')
            Image.fromarray(tile).save(inp)
            a, b = convert(inp, encoded, decoded, nd, False, normal_mode)
            timings['encodeMs'] += a; timings['decodeMs'] += b
            out = np.asarray(Image.open(decoded).convert('RGB'))
            if nd == 0 and not np.array_equal(tile, out):
                raise ValueError('法線の可逆照合に失敗しました')
            if normal_padding:blend_tile(normal_accum,normal_weights,out.astype(np.float32),x,y,cw,ch,normal_padding)
            normal[y:y+ch, x:x+cw] = out[normal_padding:normal_padding+ch,normal_padding:normal_padding+cw]
            normal_bytes += encoded.stat().st_size
            normal_tiles.append((x, y, cw, ch, encoded.read_bytes()))
            inp.unlink(); decoded.unlink()
    if normal_padding:normal=np.rint(normal_accum/normal_weights[...,None]).clip(0,255).astype(np.uint8)
    Image.fromarray(normal).save(folder/'normal.png')
    with (folder/'normal.yjn').open('wb') as stream:
        stream.write((b'YJN2'+struct.pack('<HHII',size,size,len(normal_tiles),normal_padding)) if seams=='overlap' else b'YJN1'+struct.pack('<HHI', size, size, len(normal_tiles)))
        for x, y, w, h, body in normal_tiles:
            stream.write(struct.pack('<HHHHI', x, y, w, h, len(body)))
            stream.write(body)
    error=np.where(valid,restored-heights,np.nan)
    jumps=[]
    for axis in [0,1]:
        for edge in range(256,heights.shape[axis],256):
            a=np.take(error,edge,axis=axis);b=np.take(error,edge-1,axis=axis)
            finite=np.isfinite(a)&np.isfinite(b)
            if finite.any():jumps.append(float(np.abs(a[finite]-b[finite]).max()))
    angles = np.degrees(np.arccos(np.clip((direction(original)*direction(normal)).sum(axis=-1), -1, 1)))
    result = dict(id=folder.name, region=region, heightMode=height_mode, normalMode=normal_mode, seams=seams, heightPadding=padding, normalPadding=normal_padding, level=level, normalSize=size, heightDistance=hd, normalDistance=nd,
                  heightBytes=(folder/'height.yjd').stat().st_size, heightImageBytes=height_bytes,
                  heightBoundaryErrorJump=max(jumps,default=0.), maskBytes=mask_bytes, normalBytes=(folder/'normal.yjn').stat().st_size, normalImageBytes=normal_bytes, tileSize=256, tileGrid=tile_grid, heightTileColumns=math.ceil(heights.shape[1]/256), heightTileRows=math.ceil(heights.shape[0]/256), normalTileColumns=math.ceil(size/256), normalTileRows=math.ceil(size/256),
                  maxHeightError=float(height_error.max()) if height_error.size else 0,
                  rmseHeight=float(np.sqrt(np.mean(height_error**2))) if height_error.size else 0,
                  normalP95=float(np.percentile(angles, 95)), normalMax=float(angles.max()),
                  heightURL=f'/dynamic-jxl/{folder.name}/height.f32', normalURL=f'/dynamic-jxl/{folder.name}/normal.png',
                  **timings)
    (folder/'result.json').write_text(json.dumps(result), encoding='utf-8')
    return result
