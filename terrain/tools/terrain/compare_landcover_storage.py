"""平滑化なしの土地被覆を同じ面積・解像度で圧縮比較する。"""
import io,json,sqlite3,zlib,time
import numpy as np
from PIL import Image
from generate_landcover_color import ROOT,PALETTE,classes_for_area
from dynamic_jxl import convert
from fixed_normal_codec import compare

OUT=ROOT/'build/landcover-compare'
CODES=np.array(list(PALETTE),np.uint8)
COLORS=np.array(list(PALETTE.values()),np.float32)

def png(image):
    stream=io.BytesIO();image.save(stream,format='PNG',compress_level=9)
    return stream.getvalue()

def fractions(classes,size):
    # 番号自体は平均せず、各分類の被覆率を面積平均する。
    return np.stack([np.asarray(Image.fromarray((classes==c).astype(np.float32)).resize((size,size),Image.Resampling.BOX)) for c in CODES],axis=-1)

def pack_two(weights):
    indices=np.argsort(weights,axis=-1,kind='stable')[...,-2:][...,::-1].astype(np.uint8)
    picked=np.take_along_axis(weights,indices,axis=-1)
    ratio=np.rint(255*picked[...,0]/np.maximum(picked.sum(axis=-1),1e-8)).astype(np.uint8)
    return np.stack([indices[...,0]|(indices[...,1]<<4),ratio],axis=-1)

def unpack_two(packed):
    ratio=packed[...,1,None].astype(np.float32)/255
    return COLORS[packed[...,0]&15]*ratio+COLORS[packed[...,0]>>4]*(1-ratio)

def main():
    OUT.mkdir(exist_ok=True);rows=[]
    with sqlite3.connect(ROOT/'build/terrain-shared-allocation-error-detail20-direct-e800.sqlite3') as db:
        areas=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])['reliefRegions']
    for area in areas:
        region=area['id'];cache=OUT/f'{region}-classes.npy'
        if cache.exists():classes=np.load(cache)
        else:
            classes,_,_=classes_for_area(area);np.save(cache,classes)
        for size in [512,1024,2048]:
            print('比較開始',region,size,flush=True)
            weights=fractions(classes,size);reference=np.rint(weights@COLORS).clip(0,255).astype(np.uint8)
            image=Image.fromarray(reference);prefix=f'{region}-{size}'
            source=OUT/f'{prefix}-reference.png';source.write_bytes(png(image))
            def record(mode,label,body,decoded,gpu_bytes,decode_ms=0,encoded_bytes=None):
                path=f'{prefix}-{mode}.png';(OUT/path).write_bytes(png(Image.fromarray(decoded)))
                row=dict(region=region,size=size,mode=mode,label=label,bytes=len(body) if encoded_bytes is None else encoded_bytes,gpuBytes=gpu_bytes,rmse=float(np.sqrt(np.mean((decoded.astype(np.float32)-reference)**2))),preview=path,decodeMs=decode_ms)
                rows.append(row)
                if body is not None:(OUT/f'{prefix}-{mode}.bin').write_bytes(body)
            record('rgb','RGB PNG（面積平均・基準）',source.read_bytes(),reference,size*size*3)
            for count in [64,128,256]:
                quantized=image.quantize(colors=count,method=Image.Quantize.MEDIANCUT,dither=Image.Dither.NONE)
                record(f'palette{count}',f'パレット PNG {count}色',png(quantized),np.asarray(quantized.convert('RGB')),size*size)
            dominant=np.argmax(weights,axis=-1).astype(np.uint8);body=zlib.compress(dominant.tobytes(),9)
            record('class','最多分類＋Deflate',body,COLORS[dominant].astype(np.uint8),size*size)
            packed=pack_two(weights);body=zlib.compress(packed.tobytes(),9)
            started=time.perf_counter();restored=np.frombuffer(zlib.decompress(body),np.uint8).reshape(size,size,2);decoded=np.rint(unpack_two(restored)).clip(0,255).astype(np.uint8)
            record('two','2分類＋混合率＋Deflate',body,decoded,size*size*2,(time.perf_counter()-started)*1000)
            for distance in [.3,1.]:
                encoded=OUT/f'{prefix}-d{distance}.jxl';preview=OUT/f'{prefix}-d{distance}.png'
                _,ms=convert(source,encoded,preview,distance,False)
                record(f'jxl{distance}',f'JPEG XL d={distance}',encoded.read_bytes(),np.asarray(Image.open(preview).convert('RGB')),size*size*3,ms)
            # KTX2符号化器は未導入のため、GPU圧縮の比較は既存ASTC実装を使う。
            for block in [6,8]:
                preview,report=compare(source.read_bytes(),'astc',60,.3,'auto',block=block,correction=True)
                # compare() が保存した実データを使う。容器16 byteも容量へ含む。
                record(f'astc{block}',f'ASTC {block}×{block}',None,np.asarray(Image.open(io.BytesIO(preview)).convert('RGB')),report['gpuBytes'],report['decodeMs'],report['bytes'])
            (OUT/'report.json').write_text(json.dumps(dict(complete=False,rows=rows),ensure_ascii=False),encoding='utf-8')
    (OUT/'report.json').write_text(json.dumps(dict(complete=True,extentKm=128,note='追加の平滑化なし。分類被覆率の面積平均。GPU容量はベースレベルのみ。PNGのパレットは独自シェーダー展開を仮定。ASTCは符号化サイズを実測し、プレビューは復号PNG。',rows=rows),ensure_ascii=False,indent=2),encoding='utf-8')
    print('比較完了',len(rows),flush=True)

if __name__=='__main__':main()
