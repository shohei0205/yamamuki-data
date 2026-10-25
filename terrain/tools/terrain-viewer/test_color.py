"""HTMLと同じ平面色がネイティブ画面に出るかを実画像で検証する。"""
import argparse
from collections import Counter
from pathlib import Path
import sqlite3
import struct
import subprocess
from PIL import Image

parser=argparse.ArgumentParser()
parser.add_argument('--source',type=Path,required=True)
parser.add_argument('--binary',type=Path,required=True)
parser.add_argument('--output',type=Path,required=True)
args=parser.parse_args()
args.output.mkdir(parents=True,exist_ok=True)
fixture=args.output/'flat-z6.sqlite3'
if fixture.exists():
    raise SystemExit('既存の試験SQLiteを上書きしません: '+str(fixture))
with sqlite3.connect(args.source.resolve().as_uri()+'?mode=ro',uri=True) as source, sqlite3.connect(fixture) as target:
    schema=source.execute("SELECT sql FROM sqlite_master WHERE name='tiles'").fetchone()[0]
    target.execute(schema)
    for row in source.execute('SELECT * FROM tiles WHERE z=6'):
        # 列番号を固定せず元のテーブル定義を使う。
        columns=[r[1] for r in source.execute('PRAGMA table_info(tiles)')]
        row=list(row)
        at=columns.index('vertices')
        data=bytearray(row[at])
        count,deps,refs,added,size=struct.unpack_from('<5I',data,8)
        assert deps==refs==size==0 and count==added
        for i in range(added):
            struct.pack_into('<H',data,32+6*i+4,24000)
        row[at]=bytes(data)
        target.execute('INSERT INTO tiles VALUES('+','.join('?' for _ in row)+')',row)
image=args.output/'flat-z6.png'
result=subprocess.run([str(args.binary.resolve()),str(fixture.resolve()),'--smoke-output',str(image.resolve())],capture_output=True,timeout=30)
(args.output/'stdout.log').write_bytes(result.stdout)
(args.output/'stderr.log').write_bytes(result.stderr)
assert result.returncode==0,result.stderr.decode('utf-8',errors='replace')
with Image.open(image) as picture:
    cropped=picture.convert('RGB').crop((0,200,picture.width,picture.height))
    colors=Counter(getattr(cropped,'get_flattened_data',cropped.getdata)())
    # HTMLの平面: base(.40,.49,.32)*shade(.82)。空はclearColor(.90,.945,.98)。
    for name,expected in [('地表',(84,102,67)),('背景',(230,241,250))]:
        pixels=sum(n for color,n in colors.items() if max(abs(a-b) for a,b in zip(color,expected))<=1)
        assert pixels>1000,(name,expected,pixels,colors.most_common(5))
        print(name,expected,'一致画素数',pixels)
print('HTML色の画素検証: 成功')
