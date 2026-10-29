"""元DEM・800m版の保存XYZへ、既存の全域傾き調整C1を適用する。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','1')
from pathlib import Path
from contextlib import closing
from concurrent.futures import ThreadPoolExecutor
import sqlite3,json,time,hashlib
import numpy as np
from sibson_global_c1 import GlobalC1
from test_fixed_lpf_regions import ROOT,write
RUN=ROOT/'build/natural-raw-shape800-v1'
def run(name):
 folder=RUN/name;start=time.perf_counter();center=np.asarray(json.loads((folder/'region.json').read_text())['center'])
 with closing(sqlite3.connect((folder/'work.sqlite3').as_uri()+'?mode=ro',uri=True)) as c:points=np.asarray(c.execute('SELECT p.qx,p.qy,l.qz FROM points p JOIN point_levels l ON l.id=p.id WHERE l.z=8 ORDER BY p.id').fetchall(),dtype=np.float64)*[.0625,.0625,.125]
 points[:,:2]-=center;before=points.copy();stamp=hashlib.sha256(points.astype('<f8').tobytes()).hexdigest()
 def progress(info):write(folder/'c1-progress.json',dict(region=name,**info,elapsed=time.perf_counter()-start,complete=False))
 progress(dict(stage='fitting-global-gradients'));model=GlobalC1(points,strength=.01,progress=progress)
 axis=np.linspace(-8000,8000,513);xx,yy=np.meshgrid(axis,axis);q=np.column_stack((xx.ravel(),yy.ravel()));progress(dict(stage='evaluating-c1-surface',fit=model.fit_stats));h=model(q);gx=(model(q+[1.,0])-model(q-[1.,0]))/2;gy=(model(q+[0,1.])-model(q-[0,1.]))/2
 if not np.isfinite(np.column_stack([h,gx,gy])).all():raise ValueError('C1曲面の表示値が不正です')
 np.testing.assert_array_equal(points,before);node_error=float(abs(model(points[:,:2])-points[:,2]).max())
 if node_error>1e-6:raise ValueError(f'C1が保存点の標高を変更しました: {node_error}')
 dem=np.fromfile(folder/'800-dem.bin',dtype='<f4').reshape(-1,3)[:,0];normal=np.fromfile(folder/'800-fit.bin',dtype='<f4').reshape(-1,3)[:,0];valid=np.isfinite(dem);difference=h[valid]-dem[valid]
 np.column_stack([h,gx,gy]).astype('<f4').tofile(folder/'800-c1.bin');np.savez_compressed(folder/'800-c1-gradient-cache.npz',points=points,gradient=model.gradient)
 report=dict(region=name,points=len(points)-4,sigma=0,fit_error_m=None,max_error_guaranteed=False,source_xyz_sha256=stamp,control_point_max_error=node_error,rawRmse=float(np.sqrt(np.mean(difference**2))),rawMaxError=float(abs(difference).max()),normalDifferenceMax=float(abs(h-normal).max()),normalDifferenceRmse=float(np.sqrt(np.mean((h-normal)**2))),fit=model.fit_stats,seconds=time.perf_counter()-start,measurement_scope='中心16kmの513x513表示格子',geometry='Sibson-global-C1',strength=.01,complete=True)
 write(folder/'preview-800-c1.json',report);write(folder/'c1-progress.json',dict(stage='complete',complete=True,seconds=report['seconds']));return report
if __name__=='__main__':
 with ThreadPoolExecutor(max_workers=3) as pool:reports=list(pool.map(run,['fuji','yari','tsukuba']))
 write(RUN/'c1-complete.json',dict(complete=True,reports=reports))
