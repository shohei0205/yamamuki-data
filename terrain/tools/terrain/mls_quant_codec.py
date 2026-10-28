"""独立MLSの固定小数点・空間順予測・圧縮を比較する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import json,sqlite3,zlib,lzma,struct
from pathlib import Path
import numpy as np
from scipy.stats import qmc
from free_mls import LocalSupport,operator,reference_values,SPAN
from mls_error_quality import peak_samples,requirements
HEADER=struct.Struct('<4sIffB')
def encode_quant(p,xy=.25,zstep=.125,codec=1):
 p=np.asarray(p,dtype=float).reshape(-1,3)
 if xy<=0 or zstep<=0 or not np.isfinite(p).all():raise ValueError('不正な座標または刻み')
 rounded=np.rint(p/np.array([xy,xy,zstep]))
 if np.any(abs(rounded)>np.iinfo(np.int32).max):raise ValueError('座標の範囲外')
 v=rounded.astype('<i4');coords=v[:,:2].astype(np.int64)+1000000
 if np.any(coords<0) or np.any(coords>=2**22):raise ValueError('空間順の範囲外')
 morton=np.zeros(len(v),np.uint64)
 for bit in range(22):
  morton|=((coords[:,0]>>bit)&1).astype(np.uint64)<<(bit*2);morton|=((coords[:,1]>>bit)&1).astype(np.uint64)<<(bit*2+1)
 v=v[np.argsort(morton,kind='stable')];d=np.diff(v,axis=0,prepend=np.zeros((1,3),dtype='<i4')).astype('<i4');raw=d.view('u1').reshape(-1,12).T.copy().tobytes()
 packed=lzma.compress(raw,preset=6) if codec else zlib.compress(raw,9)
 return HEADER.pack(b'MLQ1',len(v),xy,zstep,codec)+packed

def decode_quant(blob):
 if len(blob)<HEADER.size:raise ValueError('ヘッダーが不足しています')
 magic,n,xy,z,codec=HEADER.unpack_from(blob)
 if not np.isfinite([xy,z]).all() or xy<=0 or z<=0:raise ValueError('不正な刻み')
 if magic!=b'MLQ1' or codec not in (0,1):raise ValueError('不正な形式')
 raw=(lzma.decompress if codec else zlib.decompress)(blob[HEADER.size:])
 if len(raw)!=12*n:raise ValueError('不正な点数')
 d=np.frombuffer(raw,'u1').reshape(12,n).T.copy().view('<i4').reshape(n,3)
 return (np.cumsum(d.astype(np.int64),axis=0)*[xy,xy,z]).astype('<f4')

def main():
 out=Path('build/mls-quant-codec-v1');out.mkdir(exist_ok=True)
 axis=np.linspace(-16000,16000,257);x,y=np.meshgrid(axis,axis);q=np.vstack((np.column_stack((x.ravel(),y.ravel())),(qmc.Sobol(2,scramble=True,seed=904).random_base2(14)-.5)*SPAN,peak_samples(12,935,1000)))
 h=np.vstack(((qmc.Sobol(2,scramble=True,seed=1904).random_base2(14)-.5)*SPAN,peak_samples(12,1935,1000)))
 with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:b=db.execute("SELECT coefficients FROM surfaces WHERE region='fuji' AND n=256").fetchone()[0]
 original=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261);truth=reference_values(original,q);ht=reference_values(original,h);report=[]
 with sqlite3.connect('build/mls-error-quality-v7/quality.sqlite3') as db,sqlite3.connect(out/'quality.sqlite3') as dest:
  dest.execute('CREATE TABLE IF NOT EXISTS levels(error REAL PRIMARY KEY,info TEXT,points BLOB)')
  for e,b in db.execute("SELECT target_error,points FROM levels WHERE region='fuji' ORDER BY target_error DESC"):
   p=np.frombuffer(zlib.decompress(b),'<f4')[1:].reshape(-1,3);blobs=[encode_quant(p,codec=i) for i in (0,1)];choice=min(blobs,key=len);restored=decode_quant(choice);s=LocalSupport(restored[:,:2],pre_smooth=False,floor_factor=.5)
   info=dict(error=e,controls=len(p),oldBytes=len(b),zlibBytes=len(blobs[0]),lzmaBytes=len(blobs[1]),bytes=len(choice),audit=requirements(operator(restored[:,:2],q,s,curvature=1e-5)@restored[:,2]-truth,q,e),heldout=requirements(operator(restored[:,:2],h,s,curvature=1e-5)@restored[:,2]-ht,h,e))
   report.append(info);dest.execute('INSERT INTO levels VALUES(?,?,?)',(e,json.dumps(info),choice));dest.commit();print(json.dumps(info),flush=True)
 (out/'report.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
if __name__=='__main__':main()
