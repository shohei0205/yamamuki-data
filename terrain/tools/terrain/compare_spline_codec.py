"""既存スプラインを変更せず、可逆・量子化・親差分の保存容量を比較する。"""
import argparse
import json
import sqlite3
import time
import zlib
from pathlib import Path

import numpy as np
from bspline_surface import HALO,SPAN,load,flatten,evaluate


def shuffle(raw):
    return np.frombuffer(raw,np.uint8).reshape(-1,4).T.copy().tobytes()


def unshuffle(raw):
    if not raw:return b''
    return np.frombuffer(raw,np.uint8).reshape(4,-1).T.copy().tobytes()


def gaps(indices):
    out=bytearray();previous=0
    for value in indices:
        delta=int(value)-previous;previous=int(value)
        while delta>=128:out.append((delta&127)|128);delta>>=7
        out.append(delta)
    return bytes(out)


def ungaps(raw):
    result=[];value=shift=previous=0
    for byte in raw:
        value|=(byte&127)<<shift
        if byte&128:shift+=7
        else:previous+=value;result.append(previous);value=shift=0
    if shift:raise ValueError('位置番号が途中で終わっています')
    return np.array(result,'<u4')


def predict_integer(parent,n):
    """3次の節点挿入。整数の分子を求め、2軸を合わせて一度だけ丸める。"""
    positions=np.arange(n+5);center=positions//2+1
    indices=np.column_stack((center-1,center,center+1))
    weights=np.where((positions%2==0)[:,None],np.array([1,6,1]),np.array([0,4,4])).astype(np.int64)
    horizontal=sum(parent[:,indices[:,k]].astype(np.int64)*weights[:,k] for k in range(3))
    numerator=sum(horizontal[indices[:,k],:]*weights[:,k,None] for k in range(3))
    return ((numerator+32)//64).astype('<i4')


def predict_float(parent,n):
    positions=np.arange(n+5);center=positions//2+1
    indices=np.column_stack((center-1,center,center+1))
    weights=np.where((positions%2==0)[:,None],np.array([1,6,1]),np.array([0,4,4]))
    horizontal=sum(parent[:,indices[:,k]].astype(np.float64)*weights[:,k] for k in range(3))
    return (sum(horizontal[indices[:,k],:]*weights[:,k,None] for k in range(3))/64).astype('<f4')


def zigzag(value):
    value=value.astype(np.int64)
    return ((value<<1)^(value>>31)).astype('<u4')


def unzigzag(value):
    return ((value>>1).astype(np.int64)^-(value&1).astype(np.int64)).astype('<i4')


METHODS={
 'current':'現行・階層残差',
 'gap-shuffle':'階層残差＋位置差分＋バイト整列',
 'gap-xor':'階層残差＋位置差分＋係数 XOR',
 'independent-f32':'独立・float32（必要範囲のみ）',
 'parent-xor':'親係数参照＋float32 XOR（可逆）',
 'parent-refine-xor':'親の節点挿入予測＋float32 XOR（可逆）',
 'independent-q025':'独立・係数 25 cm 刻み',
 'independent-q025-zz':'独立・25 cm＋ZigZag',
 'parent-q025':'親の節点挿入予測＋整数差分・25 cm',
 'parent-q025-zz':'親の節点挿入予測＋整数差分・25 cm＋ZigZag',
 'checkpoint-q025':'25 cm＋親差分・独立段を挿入（最大 3 段取得）',
}


def verify_database(path,source_path):
    """保存済み BLOB と復号済みの親だけで全階層を復元する。"""
    with sqlite3.connect(Path(path).resolve().as_uri()+'?mode=ro',uri=True) as db,sqlite3.connect(Path(source_path).resolve().as_uri()+'?mode=ro',uri=True) as source:
        restored={};count=0
        for region,n,method,raw,ib,cb in db.execute('SELECT * FROM encodings ORDER BY region,n,method'):
            info=json.loads(raw);blob=zlib.decompress(cb);codec=info.get('codec',method)
            if method in ['current','gap-shuffle','gap-xor']:
                ix=np.frombuffer(zlib.decompress(ib),'<u4') if method=='current' else ungaps(zlib.decompress(ib))
                bits=np.frombuffer(blob if method=='current' else unshuffle(blob),'<u4')
                if method=='gap-xor':bits=np.bitwise_xor.accumulate(bits)
                _,layers=load(source,region,n);target=layers[-1][1].ravel();expected=np.flatnonzero(target).astype('<u4')
                np.testing.assert_array_equal(ix,expected);np.testing.assert_array_equal(bits,target[expected].view('<u4'))
            else:
                bits=np.frombuffer(unshuffle(blob),'<u4').reshape(n+5,n+5);parent=restored.get((region,n//2,method)) if info['parent'] is not None else None
                if codec=='parent-xor' and parent is not None:bits=bits^parent[np.ix_(np.arange(n+5)//2+1,np.arange(n+5)//2+1)]
                if codec=='parent-refine-xor' and parent is not None:bits=bits^predict_float(parent.view('<f4'),n).view('<u4')
                if 'q025' in method:
                    bits=unzigzag(bits) if codec.endswith('-zz') else bits.view('<i4')
                    if codec.startswith('parent-q025') and parent is not None:bits=bits+predict_integer(parent,n)
                restored[(region,n,method)]=bits
                _,layers=load(source,region,n);dense=flatten(layers)[HALO-2:HALO+n+3,HALO-2:HALO+n+3]
                expected=np.floor(dense.astype(float)*4+.5).astype('<i4') if 'q025' in method else dense.view('<u4')
                np.testing.assert_array_equal(bits,expected)
            assert len(ib)+len(cb)==info['bytes'];count+=1
    return count


def add_checkpoints(path,rows):
    method='checkpoint-q025';anchors={8,64,256}
    with sqlite3.connect(path) as db:
        for region in sorted({r['region'] for r in rows}):
            total=required=0
            for n in [8,16,32,64,128,256,512]:
                codec='independent-q025' if n in anchors else 'parent-q025-zz'
                raw,ib,cb=db.execute('SELECT info,positions,coefficients FROM encodings WHERE region=? AND n=? AND method=?',(region,n,codec)).fetchone()
                row=json.loads(raw);size=len(ib)+len(cb);total+=size;required=size if n in anchors else required+size
                row.update(method=method,label=METHODS[method],codec=codec,requiredBytes=required,allLevelsBytes=total,parent=None if n in anchors else n//2,dependency='独立段から最大 2 回の親参照')
                db.execute('INSERT INTO encodings VALUES(?,?,?,?,?,?)',(region,n,method,json.dumps(row),ib,cb));rows.append(row)


def write_html(report,path):
    html='''<!doctype html><meta charset="utf-8"><title>B スプラインの差分圧縮比較</title>
<style>body{font:16px system-ui;margin:28px;color:#25313b}table{border-collapse:collapse;margin:12px 0 30px}td,th{border:1px solid #ccd4da;padding:9px;text-align:right}td:first-child{text-align:left}th{background:#edf2f6}.bar{display:inline-block;height:8px;background:#70a6c8;vertical-align:middle;margin-right:8px}select{font:inherit;padding:5px}small{color:#53616b}</style>
<h1>B スプラインの差分圧縮比較</h1><p>既存 3 地域・全 7 階層・7 方式。147 件を保存した SQLite から親を使って復元し、全件一致を確認。既存の表示データは変更していません。</p>
<p>細かさ <select id="level"></select>　単位 KiB（1,024 bytes）・zlib level 6 の BLOB 容量</p>
<p>「復元に必要」は親を含む取得量。「全階層」は選択した細かさまで全部保存した量。独立方式は 1 段だけで復元できます。追加差は現行 GPU 用 float32 曲面との差です。元 DEM との差は含みません。</p>
<p>25 cm は標高画素ではなく<strong>制御係数の刻み</strong>。基底が非負で重みの和が 1 のため、曲面の追加標高差は最大 12.5 cm に抑えられます。実測はランダム 2,048 点。SQLite の管理領域・JSON の説明列・通信ヘッダーは集計対象外。</p><div id="tables"></div><small>親 XOR は親の最寄り係数ビットを予測値に使用。整数差分は 3 次 B スプラインの節点挿入を整数演算で予測。現行の階層残差とは保存・復元経路が異なります。</small>
<script>const report=__REPORT__,level=document.getElementById('level');for(const n of [8,16,32,64,128,256,512])level.add(new Option(`${32000/n} m 区間（n=${n}）`,n));level.value='128';function render(){const rows=report.rows.filter(r=>r.n===+level.value);let html='';for(const region of ['合計','fuji','yari','tsukuba']){const values=region==='合計'?Object.keys(report.methods).map(method=>{const a=rows.filter(r=>r.method===method);return{label:report.methods[method],bytes:a.reduce((s,r)=>s+r.bytes,0),requiredBytes:a.reduce((s,r)=>s+r.requiredBytes,0),allLevelsBytes:a.reduce((s,r)=>s+r.allLevelsBytes,0),extraMaxError:Math.max(...a.map(r=>r.extraMaxError))}}):rows.filter(r=>r.region===region);const base=values[0].requiredBytes;html+=`<h2>${({fuji:'富士山',yari:'槍ヶ岳',tsukuba:'筑波山'})[region]||region}</h2><table><tr><th>方式</th><th>この段の BLOB</th><th>復元に必要</th><th>現行に対する割合</th><th>全階層の保存量</th><th>追加最大差</th></tr>`;for(const r of values)html+=`<tr><td>${r.label}</td><td>${(r.bytes/1024).toFixed(1)}</td><td>${(r.requiredBytes/1024).toFixed(1)}</td><td><span class="bar" style="width:${Math.min(130,100*r.requiredBytes/base)}px"></span>${(100*r.requiredBytes/base).toFixed(1)}%</td><td>${(r.allLevelsBytes/1024).toFixed(1)}</td><td>${r.extraMaxError.toFixed(4)} m</td></tr>`;html+='</table>'}document.getElementById('tables').innerHTML=html}level.onchange=render;render();</script>'''
    html=html.replace('7 方式。147 件',f'{len(report["methods"])} 方式。{report["verifiedRows"]} 件')
    path.write_text(html.replace('__REPORT__',json.dumps(report,ensure_ascii=False)),encoding='utf-8')


def compare(args):
    args.output.mkdir(parents=True,exist_ok=True);target=args.output/'codecs.sqlite3'
    if target.exists():raise ValueError('新しい出力先を指定してください')
    source=sqlite3.connect(args.input.resolve().as_uri()+'?mode=ro',uri=True);out=sqlite3.connect(target)
    out.execute('CREATE TABLE encodings(region TEXT,n INTEGER,method TEXT,info TEXT,positions BLOB,coefficients BLOB,PRIMARY KEY(region,n,method)) WITHOUT ROWID')
    rows=[];rng=np.random.default_rng(625);x=rng.uniform(-SPAN/2,SPAN/2,2048);y=rng.uniform(-SPAN/2,SPAN/2,2048)
    for region, in source.execute('SELECT name FROM regions ORDER BY name'):
        parent_float=parent_int=None;cumulative={k:0 for k in METHODS}
        for n, in source.execute('SELECT n FROM layers WHERE region=? ORDER BY n',(region,)):
            info,layers=load(source,region,n);indices=np.flatnonzero(layers[-1][1]).astype('<u4');values=layers[-1][1].ravel()[indices].astype('<f4')
            dense=flatten(layers);crop=dense[HALO-2:HALO+n+3,HALO-2:HALO+n+3].copy();bits=crop.view('<u4');integer=np.floor(crop.astype(float)*4+.5).astype('<i4')
            previous_bits=np.r_[np.uint32(0),values.view('<u4')[:-1]];xor=values.view('<u4')^previous_bits
            predicted_bits=np.zeros_like(bits) if parent_float is None else parent_float.view('<u4')[np.ix_(np.arange(n+5)//2+1,np.arange(n+5)//2+1)]
            predicted_int=np.zeros_like(integer) if parent_int is None else predict_integer(parent_int,n)
            refined_bits=np.zeros_like(bits) if parent_float is None else predict_float(parent_float,n).view('<u4')
            current=source.execute('SELECT indices,coefficients FROM layers WHERE region=? AND n=?',(region,n)).fetchone()
            packets={
                'current':current,
                'gap-shuffle':(zlib.compress(gaps(indices),6),zlib.compress(shuffle(values.tobytes()),6)),
                'gap-xor':(zlib.compress(gaps(indices),6),zlib.compress(shuffle(xor.astype('<u4').tobytes()),6)),
                'independent-f32':(b'',zlib.compress(shuffle(crop.tobytes()),6)),
                'parent-xor':(b'',zlib.compress(shuffle((bits^predicted_bits).astype('<u4').tobytes()),6)),
                'parent-refine-xor':(b'',zlib.compress(shuffle((bits^refined_bits).astype('<u4').tobytes()),6)),
                'independent-q025':(b'',zlib.compress(shuffle(integer.tobytes()),6)),
                'independent-q025-zz':(b'',zlib.compress(shuffle(zigzag(integer).tobytes()),6)),
                'parent-q025':(b'',zlib.compress(shuffle((integer-predicted_int).astype('<i4').tobytes()),6)),
                'parent-q025-zz':(b'',zlib.compress(shuffle(zigzag(integer-predicted_int).tobytes()),6)),
            }
            reference=evaluate([(n,dense)],x,y)
            for method,(ib,cb) in packets.items():
                start=time.perf_counter();blob=zlib.decompress(cb)
                if method in ['current','gap-shuffle','gap-xor']:
                    ix=np.frombuffer(zlib.decompress(ib),'<u4') if method=='current' else ungaps(zlib.decompress(ib))
                    raw=blob if method=='current' else unshuffle(blob)
                    decoded=np.frombuffer(raw,'<u4')
                    if method=='gap-xor':decoded=np.bitwise_xor.accumulate(decoded)
                    np.testing.assert_array_equal(ix,indices);np.testing.assert_array_equal(decoded,values.view('<u4'))
                    error=0.;dependency='階層残差を粗い段から合計'
                else:
                    decoded=np.frombuffer(unshuffle(blob),'<u4').reshape(n+5,n+5)
                    if method=='parent-xor':decoded=decoded^predicted_bits
                    if method=='parent-refine-xor':decoded=decoded^refined_bits
                    if 'q025' in method:
                        restored=unzigzag(decoded) if method.endswith('-zz') else decoded.view('<i4')
                        if method.startswith('parent-q025'):restored=restored+predicted_int
                        np.testing.assert_array_equal(restored,integer);coefficients=restored.astype(float)*.25
                        assert np.max(abs(coefficients-crop))<=.125001
                    else:
                        np.testing.assert_array_equal(decoded,bits);coefficients=decoded.view('<f4')
                    full=np.zeros_like(dense);full[HALO-2:HALO+n+3,HALO-2:HALO+n+3]=coefficients
                    error=float(np.max(abs(evaluate([(n,full)],x,y)-reference)))
                    if 'q025' not in method:assert error<1e-8
                    else:assert error<=.1251
                    dependency='なし' if method.startswith('independent') else '直前の粗い係数配列'
                decode_ms=(time.perf_counter()-start)*1000;size=len(ib)+len(cb);cumulative[method]+=size
                required=cumulative[method] if not method.startswith('independent') else size
                row=dict(region=region,n=n,stepMeters=SPAN/n,method=method,label=METHODS[method],bytes=size,requiredBytes=required,allLevelsBytes=cumulative[method],decodeVerifyMs=decode_ms,extraMaxError=error,parent=n//2 if method.startswith('parent') and parent_float is not None else None,dependency=dependency,coefficients=int(crop.size if method.startswith(('parent','independent')) else len(values)))
                out.execute('INSERT INTO encodings VALUES(?,?,?,?,?,?)',(region,n,method,json.dumps(row),ib,cb));rows.append(row)
            parent_float=crop;parent_int=integer;out.commit();print(region,n,'検証完了',flush=True)
    out.close();add_checkpoints(target,rows);verified=verify_database(target,args.input);report=dict(input=str(args.input),methods=METHODS,rows=rows,sqliteBytes=target.stat().st_size,verifiedRows=verified)
    (args.output/'report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
    write_html(report,args.output/'index.html')


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--input',type=Path,default=Path('build/bspline-surface-prototype-v3/spline.sqlite3'));p.add_argument('--output',type=Path,default=Path('build/bspline-codec-comparison'));compare(p.parse_args())
