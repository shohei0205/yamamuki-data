"""独立アンカーを挟んだXYZ固定MLSをパッケージし、新しい点で検証する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import json,sqlite3,zlib
from pathlib import Path
import numpy as np
from scipy.stats import qmc
from free_mls import LocalSupport,operator,reference_values,SPAN
from mls_error_quality import peak_samples,requirements
from mls_quant_codec import decode_quant

def main():
 out=Path('build/mls-hybrid-selected-v2');out.mkdir(exist_ok=True);destpath=out/'quality.sqlite3'
 if destpath.exists():raise ValueError('既存出力を保持するため別名を指定してください')
 base=Path('build/mls-fixedxyz-prune-v4/quality.sqlite3');q=np.vstack(((qmc.Sobol(2,scramble=True,seed=4904).random_base2(15)-.5)*SPAN,peak_samples(13,4935,1000)))
 with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:b=db.execute("SELECT coefficients FROM surfaces WHERE region='fuji' AND n=256").fetchone()[0]
 original=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261);truth=reference_values(original,q);report=[];resolved={}
 with sqlite3.connect(base) as src,sqlite3.connect(destpath) as dest:
  dest.execute('CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT) WITHOUT ROWID')
  params=dict(format='MLQ1',region='fuji',span=32000,xyStep=.25,zStep=.125,scale=1.25,supportFloorFactor=.5,supportGridN=128,preSmooth=False,curvaturePenalty=1e-5,maxParentDepth=1,parentXYZ='immutable',independentErrors=[200,50],deltaErrors=[100,25])
  for k,v in params.items():dest.execute('INSERT INTO metadata VALUES(?,?)',(k,json.dumps(v)))
  dest.execute('CREATE TABLE levels(error REAL PRIMARY KEY,parent_error REAL,info TEXT,points BLOB)')
  for e,parent,raw,blob in src.execute('SELECT error,parent_error,info,points FROM levels ORDER BY error DESC'):
   p=decode_quant(blob)
   if parent is not None:p=np.vstack((resolved[parent],p))
   resolved[e]=p;info=json.loads(raw);info['freshValidation']=requirements(operator(p[:,:2],q,LocalSupport(p[:,:2],pre_smooth=False,floor_factor=.5),curvature=1e-5)@p[:,2]-truth,q,e)
   report.append(info);dest.execute('INSERT INTO levels VALUES(?,?,?,?)',(e,parent,json.dumps(info),blob))
 result=dict(levels=report,blobBytes=sum(v['bytes'] for v in report),sqliteBytes=destpath.stat().st_size,validationPoints=len(q),passed=all(v['freshValidation']['converged'] for v in report))
 # ファイルサイズは接続を閉じてから測定する。
 result['sqliteBytes']=destpath.stat().st_size
 (out/'report.json').write_text(json.dumps(result,indent=2),encoding='utf-8');print(json.dumps(result),flush=True)
if __name__=='__main__':main()
