"""親のXYZを固定し、追加点だけを保存するMLS差分の容量実験。"""
import os
os.environ.setdefault('OPENBLAS_NUM_THREADS','4')
import json, sqlite3, zlib, struct
from pathlib import Path
import numpy as np
from scipy.sparse.linalg import lsmr
from scipy.stats import qmc
from free_mls import LocalSupport, operator, reference_values, SPAN
from mls_error_quality import peak_samples, requirements

HEADER=struct.Struct('<4sIf')
def encode(points):
    p=np.asarray(points,dtype='<f4').reshape(-1,3)
    # 各座標のバイト列を分離してから可逆圧縮する。
    body=p.view('u1').reshape(-1,12).T.copy().tobytes()
    return HEADER.pack(b'MLD1',len(p),1.25)+zlib.compress(body,6)
def decode(blob):
    magic,n,scale=HEADER.unpack_from(blob)
    if magic!=b'MLD1':raise ValueError('形式が違います')
    raw=zlib.decompress(blob[HEADER.size:])
    if len(raw)!=n*12:raise ValueError('点数が一致しません')
    return np.frombuffer(raw,'u1').reshape(12,n).T.copy().view('<f4').reshape(n,3)
def run():
    out=Path('build/mls-xyz-delta-v1');out.mkdir(exist_ok=True)
    path=out/'delta.sqlite3'
    if path.exists():raise ValueError('出力が既にあります')
    axis=np.linspace(-SPAN/2,SPAN/2,257);x,y=np.meshgrid(axis,axis)
    audit=np.vstack((np.column_stack((x.ravel(),y.ravel())),(qmc.Sobol(2,scramble=True,seed=904).random_base2(14)-.5)*SPAN,peak_samples(12,935,1000)))
    with sqlite3.connect('build/ridge-spline-prototype-v4/spline.sqlite3') as db:b=db.execute("SELECT coefficients FROM surfaces WHERE region='fuji' AND n=256").fetchone()[0]
    original=np.frombuffer(zlib.decompress(b),'<f4').reshape(261,261);truth=reference_values(original,audit)
    with sqlite3.connect('build/mls-downgrade-v2/quality.sqlite3') as db:rows=db.execute("SELECT target_error,points FROM levels WHERE region='fuji' ORDER BY target_error DESC").fetchall()
    report=[];resolved=np.empty((0,3),'<f4');parent_ids=np.empty(0,np.uint32);previous=None
    with sqlite3.connect(path) as db:
        db.execute('CREATE TABLE levels(region TEXT,error REAL,parent_error REAL,info TEXT,added BLOB,PRIMARY KEY(region,error)) WITHOUT ROWID')
        for tolerance,b in rows:
            source=np.frombuffer(zlib.decompress(b),'<f4')[1:].reshape(-1,3)
            ids=np.load(f'build/mls-downgrade-v2/lineage-fuji-E{tolerance:g}.npy')
            new_mask=~np.isin(ids,parent_ids);new=source[new_mask].copy();all_points=np.vstack((resolved,new));n=len(resolved)
            if not set(parent_ids).issubset(set(ids)):raise ValueError('親XYが含まれていません')
            support=LocalSupport(all_points[:,:2],pre_smooth=False,floor_factor=.5)
            a=operator(all_points[:,:2],audit,support,curvature=1e-5)
            if n:
                fixed=a[:,:n]@resolved[:,2];design=a[:,n:];z=new[:,2].astype(float);weights=np.ones(len(audit))
                limit=np.where(np.linalg.norm(audit,axis=1)<=2000,tolerance*.5,tolerance)
                best=None
                for iteration in range(5):
                    residual=truth-fixed-design@z
                    delta=lsmr(design.multiply(weights[:,None]),residual*weights,damp=.15,maxiter=1000,atol=1e-7,btol=1e-7)[0]
                    candidate=z+delta;error=fixed+design@candidate-truth;score=float(np.max(abs(error)/limit))
                    if best is None or score<best[0]:best=(score,candidate.copy())
                    z=candidate;weights=np.maximum(weights,np.clip(abs(error)/limit,1,16)**2)
                new[:,2]=best[1].astype('<f4');all_points=np.vstack((resolved,new))
            error=a@all_points[:,2]-truth;measurement=requirements(error,audit,tolerance)
            blob=encode(new);full=encode(all_points);old_codec=zlib.compress(np.array([1.25],'<f4').tobytes()+all_points.astype('<f4').tobytes(),6)
            # 復号は親XYZの末尾に追加するだけ。既存点の値は変更しない。
            restored=np.vstack((resolved,decode(blob)))
            if not np.array_equal(restored,all_points):raise ValueError('差分復号が一致しません')
            info=dict(error=tolerance,controls=len(all_points),inherited=n,added=len(new),deltaBytes=len(blob),independentSameCodecBytes=len(full),independentOldCodecBytes=len(old_codec),originalIndependentBytes=len(b),**measurement)
            db.execute('INSERT INTO levels VALUES(?,?,?,?,?)',('fuji',tolerance,previous,json.dumps(info),blob));db.commit()
            report.append(info);print(json.dumps(info),flush=True)
            resolved=restored;parent_ids=np.concatenate((parent_ids,ids[new_mask]));previous=tolerance
    result=dict(levels=report,sqliteBytes=path.stat().st_size,deltaBlobBytes=sum(r['deltaBytes'] for r in report),independentSameCodecBytes=sum(r['independentSameCodecBytes'] for r in report),independentOldCodecBytes=sum(r['independentOldCodecBytes'] for r in report),originalIndependentBytes=sum(r['originalIndependentBytes'] for r in report))
    (out/'report.json').write_text(json.dumps(result,indent=2),encoding='utf-8');print(json.dumps(result),flush=True)
if __name__=='__main__':run()
