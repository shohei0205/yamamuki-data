"""同じ整数・参照順序を保持して既存形式と独自形式を実データ比較する。"""
import json,sqlite3,struct,zlib,time
from pathlib import Path
import numpy as np
from mls_quant_codec import decode_quant,HEADER as OLD_HEADER
from sibson_native_codec import encode_independent,decode_independent,encode_delta,decode_delta_arrays

ROOT=Path('build/sibson-native-codec-v1');ROOT.mkdir(exist_ok=True)
source=sqlite3.connect(Path('build/sibson-variable128-v1/terrain.sqlite3').resolve().as_uri()+'?mode=ro',uri=True);dest=sqlite3.connect(ROOT/'comparison.sqlite3');dest.execute('CREATE TABLE IF NOT EXISTS samples(region TEXT,error REAL,x INTEGER,y INTEGER,variant TEXT,parents TEXT,blob BLOB,info TEXT,PRIMARY KEY(region,error,x,y,variant))');reports=[]
for region in ['fuji','yari','tsukuba']:
 for error, in source.execute('SELECT DISTINCT error FROM tiles ORDER BY error DESC').fetchall():
  rows=source.execute('SELECT x,y,parents,blob FROM tiles WHERE region=? AND error=? ORDER BY y,x',(region,error)).fetchall();indices=np.unique(np.rint(np.linspace(0,len(rows)-1,min(24,len(rows)))).astype(int))
  for index in indices:
   x,y,parents,old=rows[index];parent_list=json.loads(parents);start=time.perf_counter()
   if old[:4]==b'MLQ1':
    _,n,xy,z,codec=OLD_HEADER.unpack_from(old);v=np.rint(decode_quant(old)/[xy,xy,z]).astype(np.int64);kind='independent';data=(v,xy,z);baseline='LZMA' if codec else 'zlib'
   else:
    _,n,np_,nf=struct.unpack_from('<4sIII',old);raw=zlib.decompress(old[16:]);refs=np.frombuffer(raw[:4*n],np.uint8).reshape(4,n).T.copy().view('<u4').ravel();dz=np.frombuffer(raw[4*n:8*n],np.uint8).reshape(4,n).T.copy().view('<i4').ravel();fresh=np.frombuffer(raw[8*n:],np.uint8).reshape(4,3*nf).T.copy().view('<i4').reshape(nf,3);kind='delta';data=(refs,dz,fresh,np_);baseline='zlib'
   baseline_ms=(time.perf_counter()-start)*1000
   for rice,name in [(False,'NQ-block'),(True,'NQ-rice')]:
    start=time.perf_counter();blob=encode_independent(data[0],data[1],data[2],rice) if kind=='independent' else encode_delta(*data,rice=rice);encode_ms=(time.perf_counter()-start)*1000;start=time.perf_counter()
    restored=decode_independent(blob) if kind=='independent' else decode_delta_arrays(blob)
    decode_ms=(time.perf_counter()-start)*1000
    if kind=='independent':np.testing.assert_array_equal(restored,data[0])
    else:
     for actual,expected in zip(restored,data[:3]):np.testing.assert_array_equal(actual,expected)
    item=dict(region=region,error=error,x=x,y=y,kind=kind,variant=name,points=n,baseline=baseline,baselineBytes=len(old),bytes=len(blob),baselineDecodeMs=baseline_ms,encodeMs=encode_ms,decodeMs=decode_ms);reports.append(item);dest.execute('INSERT OR REPLACE INTO samples VALUES(?,?,?,?,?,?,?,?)',(region,error,x,y,name,parents,blob,json.dumps(item)))
  dest.commit();print(region,error,len(indices),'samples',flush=True)
dest.close();(ROOT/'report.json').write_text(json.dumps(reports,indent=2),encoding='utf-8');summary=[]
for kind in ['independent','delta','all']:
 for variant in ['NQ-block','NQ-rice']:
  rows=[r for r in reports if r['variant']==variant and (kind=='all' or r['kind']==kind)];a=sum(r['baselineBytes'] for r in rows);b=sum(r['bytes'] for r in rows);item=dict(kind=kind,variant=variant,tiles=len(rows),baselineBytes=a,bytes=b,ratio=b/a,decodeMs=sum(r['decodeMs'] for r in rows),baselineDecodeMs=sum(r['baselineDecodeMs'] for r in rows));summary.append(item);print(item,flush=True)
(ROOT/'summary.json').write_text(json.dumps(summary,indent=2),encoding='utf-8')
