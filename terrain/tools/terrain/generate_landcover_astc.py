"""地表色をASTC 8×8の全ミップへ事前圧縮する。平滑化は追加しない。"""
import hashlib,io,json,sqlite3,struct,subprocess
from pathlib import Path
from PIL import Image
from national_independent_mesh import ROOT

OUTPUT=ROOT/'build/terrain-landcover-astc.sqlite3'
CACHE=ROOT/'build/terrain-landcover-astc'
TOOL=ROOT/'build/terrain-codec-tools/astc/bin/astcenc-avx2.exe'

def astc_payload(body,size):
    if len(body)<16 or body[:7]!=bytes.fromhex('13aba15c080801'):
        raise ValueError('ASTC 8×8のヘッダーが不正です')
    dimensions=[int.from_bytes(body[i:i+3],'little') for i in [7,10,13]]
    expected=((size+7)//8)**2*16
    if dimensions!=[size,size,1] or len(body)!=expected+16:
        raise ValueError('ASTCの寸法または長さが不正です')
    return body[16:]

def bundle(size,levels,magic):
    return struct.pack('<4sII',magic,size,len(levels))+b''.join(struct.pack('<III',n,n,len(body))+body for n,body in levels)

def run(*args):
    subprocess.run([str(TOOL),*map(str,args)],check=True,capture_output=True,timeout=180,creationflags=subprocess.CREATE_NO_WINDOW)

def encode(image):
    stream=io.BytesIO();image.save(stream,format='PNG');body=stream.getvalue()
    folder=CACHE/hashlib.sha256(body+b'astc8-60-rgba-v1').hexdigest();folder.mkdir(parents=True,exist_ok=True)
    source=folder/'source.png';encoded=folder/'image.astc';preview=folder/'decoded.png'
    if not encoded.exists():
        source.write_bytes(body);run('-cl',source,encoded,'8x8','60','-j','4','-silent')
    payload=astc_payload(encoded.read_bytes(),image.width)
    if not preview.exists():run('-dl',encoded,preview,'-j','4','-silent')
    return payload,preview.read_bytes()

def main():
    CACHE.mkdir(exist_ok=True)
    with sqlite3.connect(ROOT/'build/terrain-landcover-color.sqlite3') as source,sqlite3.connect(OUTPUT) as target:
        target.execute('CREATE TABLE IF NOT EXISTS images(region TEXT,size INTEGER,codec TEXT,data BLOB,PRIMARY KEY(region,size,codec))')
        target.execute('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT)')
        for region,size,body in source.execute('SELECT region,size,data FROM images ORDER BY region,size'):
            image=Image.open(io.BytesIO(body)).convert('RGB');compressed=[];previews=[];n=size
            while n>=1:
                # 各ミップは非圧縮の基底画像から面積平均し、圧縮劣化の蓄積を避ける。
                mip=image if n==size else image.resize((n,n),Image.Resampling.BOX)
                data,preview=encode(mip);compressed.append((n,data));previews.append((n,preview));n//=2
            for codec,magic,levels in [('astc8',b'LCA1',compressed),('astc8preview',b'LCP1',previews)]:
                data=bundle(size,levels,magic)
                target.execute('INSERT OR REPLACE INTO images VALUES(?,?,?,?)',(region,size,codec,data))
            target.commit();print(region,size,'ASTC',sum(len(b) for _,b in compressed),'bytes',len(compressed),'mips',flush=True)
        target.execute('INSERT OR REPLACE INTO metadata VALUES("manifest",?)',(json.dumps(dict(version=1,block=[8,8],mipmaps=True,extraSmoothing=False,format='LCA1',colorSpace='unorm-same-as-png',source='terrain-landcover-color.sqlite3')),))
    print('地表ASTC全ミップ生成完了',flush=True)

if __name__=='__main__':main()
