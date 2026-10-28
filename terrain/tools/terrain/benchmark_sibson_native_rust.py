"""現行Python API（ネイティブ圧縮ライブラリー使用）とRustのXYZ復元を比較。"""
import base64,ctypes,json,sqlite3,time,statistics
from pathlib import Path
import numpy as np
from mls_quant_codec import decode_quant
from sibson_variable128 import restore,decode_subset
from sibson_variable_server import Regional

ROOT=Path('build/sibson-native-codec-v1');lib=ctypes.CDLL(str((ROOT/'sibson_native_decoder.dll').resolve()));lib.codec_decode.argtypes=[ctypes.c_void_p,ctypes.c_size_t,ctypes.c_void_p,ctypes.c_size_t,ctypes.c_void_p,ctypes.c_size_t,ctypes.c_void_p,ctypes.c_size_t];lib.codec_decode.restype=ctypes.c_int
source=sqlite3.connect(Path('build/sibson-variable128-v1/terrain.sqlite3').resolve().as_uri()+'?mode=ro',uri=True);samples=sqlite3.connect(ROOT/'comparison.sqlite3');cases=[];memo={};corpus=[]
def b64(blob):return base64.b64encode(blob).decode()
for region,error,x,y in samples.execute('SELECT DISTINCT region,error,x,y FROM samples ORDER BY region,error DESC,y,x'):
 parents,old=source.execute('SELECT parents,blob FROM tiles WHERE region=? AND error=? AND x=? AND y=?',(region,error,x,y)).fetchone();refs=json.loads(parents);m=memo.setdefault(region,{});arrays=[restore(Regional(source,region),*ref,memo=m) for ref in refs];offsets=np.array([0,*np.cumsum([len(p) for p in arrays])],np.uint32);flat=np.vstack(arrays).astype('<f4') if arrays else np.empty((0,3),'<f4');expected=decode_subset(old,arrays) if refs else decode_quant(old);variants=dict(samples.execute('SELECT variant,blob FROM samples WHERE region=? AND error=? AND x=? AND y=?',(region,error,x,y)));item=dict(region=region,error=error,x=x,y=y,old=old,parents=arrays,offsets=offsets,flat=flat,expected=expected,variants=variants,out=np.empty_like(expected));cases.append(item)
 corpus.append(dict(region=region,error=error,x=x,y=y,n=len(expected),kind='delta' if refs else 'independent',oldBytes=len(old),parents=b64(flat.tobytes()),offsets=offsets.tolist(),expected=b64(expected.tobytes()),variants={k:b64(v) for k,v in variants.items()}))
(ROOT/'wasm-corpus.json').write_text(json.dumps(corpus,separators=(',',':')),encoding='utf-8')
def decode(case,variant):
 if variant=='baseline':return decode_subset(case['old'],case['parents']) if case['parents'] else decode_quant(case['old'])
 blob=case['variants'][variant];result=lib.codec_decode(blob,len(blob),case['flat'].ctypes.data,case['flat'].size,case['offsets'].ctypes.data,len(case['parents']),case['out'].ctypes.data,len(case['out']))
 if result!=len(case['out']):raise ValueError(('Rust復元エラー',case['region'],case['error'],case['x'],case['y'],result))
 return case['out']
for case in cases:
 for variant in ['NQ-block','NQ-rice']:np.testing.assert_array_equal(decode(case,variant),case['expected'])
 # 全タイルの末尾を切断し、復元器が失敗を返すことも確認する。
 blob=case['variants']['NQ-rice'][:-1];result=lib.codec_decode(blob,len(blob),case['flat'].ctypes.data,case['flat'].size,case['offsets'].ctypes.data,len(case['parents']),case['out'].ctypes.data,len(case['out']));assert result<0
reports=[]
for kind in ['independent','delta','all']:
 selected=[c for c in cases if kind=='all' or bool(c['parents'])==(kind=='delta')]
 for variant in ['baseline','NQ-block','NQ-rice']:
  for c in selected:decode(c,variant)
  times=[]
  for _ in range(9):
   start=time.perf_counter()
   for _ in range(5):
    for c in selected:decode(c,variant)
   times.append((time.perf_counter()-start)*1000/5)
  item=dict(kind=kind,variant=variant,tiles=len(selected),points=sum(len(c['expected']) for c in selected),bytes=sum(len(c['old'] if variant=='baseline' else c['variants'][variant]) for c in selected),medianMs=statistics.median(times),minimumMs=min(times),maximumMs=max(times));reports.append(item);print(item,flush=True)
(ROOT/'native-results.json').write_text(json.dumps(reports,indent=2),encoding='utf-8');print('全702パケットのXYZ完全一致・351切断パケットを拒否',flush=True)
