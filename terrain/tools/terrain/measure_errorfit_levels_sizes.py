"""誤差適合比較111タイルの点列圧縮とGPUパケットを実測する。"""
import json,time
from pathlib import Path
import numpy as np
from test_fixed_lpf_regions import ROOT,write
from sibson_native_codec import encode_independent,decode_independent
OUT=ROOT/'build/natural-raw-c1-128-errorfit-levels-v1';BASE=ROOT/'build/natural-raw-c1-128-v1'
def run():
 rows=[];started=time.monotonic()
 for path in sorted(OUT.glob('*/*/*/selection.json')):
  folder=path.parent;region,e,xy=folder.relative_to(OUT).parts;error=float(e);info=json.loads((folder/'packet.json').read_text());p=np.load(folder/'control-gradient.npz')['points'];world=np.asarray(json.loads((BASE/region/'source/region.json').read_text())['origin'])+np.asarray(info['origin']);origin=np.rint(world/.0625).astype(np.int64);q=np.empty(p.shape,np.int64);q[:,:2]=np.rint((p[:,:2]+world)/.0625).astype(np.int64)-origin;q[:,2]=np.rint(p[:,2]/.125).astype(np.int64);blob=encode_independent(q);assert np.array_equal(q,decode_independent(blob));(folder/'control-xyz.nqc1').write_bytes(blob)
  original=BASE/region/e/xy;rows.append(dict(region=region,error=error,side=info['side'],points=len(p)-4,riceBytes=len(blob),gpuBytes=(folder/'packet.bin').stat().st_size,originalGpuBytes=(original/'packet.bin').stat().st_size,xyzOriginQ=origin.tolist()))
 levels=[]
 for error in [800,400,200,100,50,25,12.5]:
  a=[r for r in rows if r['error']==error];levels.append(dict(error=error,side=a[0]['side'],tiles=len(a),points=sum(r['points'] for r in a),riceBytes=sum(r['riceBytes'] for r in a),gpuBytes=sum(r['gpuBytes'] for r in a),originalGpuBytes=sum(r['originalGpuBytes'] for r in a)))
 report=dict(tiles=len(rows),levels=levels,rows=rows,seconds=time.monotonic()-started,note='Rice NQC1=XYZのみ独立圧縮。XY 0.0625m/Z 0.125m、現在の点順を保持。4個の遠方補助点も符号化、点数表示はそれらを除く。レベル間差分なし。GPU packet=制御点・勾配・Voronoi・探索木・ヘッダー込み、リクエスト時のvalidityMaskとHTTPヘッダーは除く。原本との比較は同じ111タイル。元DEM・SQLite・作業キャッシュ除外。')
 write(OUT/'size-report.json',report)
 for r in levels:print(json.dumps(r))
 print('TOTAL',json.dumps({k:sum(r[k] for r in levels) for k in ['points','riceBytes','gpuBytes','originalGpuBytes']}))
if __name__=='__main__':run()
