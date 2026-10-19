"""曲面の高さ・傾き誤差で頂点を選び、C1補間パッチを保存する比較生成器。"""
import json,sqlite3,struct,zlib
from pathlib import Path
import numpy as np
from scipy.interpolate import CloughTocher2DInterpolator
from scipy.ndimage import map_coordinates
ROOT=Path(__file__).resolve().parents[2]
POWERS=[(0,0),(1,0),(0,1),(2,0),(1,1),(0,2),(3,0),(2,1),(1,2),(0,3)]

def basis(uv):
    u,v=uv[...,0],uv[...,1]
    return np.stack([u**a*v**b for a,b in POWERS],axis=-1)

def pack_surface(interp,extent,n):
    vertices=interp.tri.points;faces=interp.tri.simplices
    nodes=np.array([(i/3,j/3) for i in range(4) for j in range(4-i)])
    corners=np.array([[0.,0.],[1.,0.],[0.,1.]])
    center=np.array([1/3,1/3]);coeff=[]
    for opposite in range(3):
        edge=[i for i in range(3) if i!=opposite]
        uv=corners[edge[0]]+(corners[edge[1]]-corners[edge[0]])*nodes[:,0,None]+(center-corners[edge[0]])*nodes[:,1,None]
        xy=vertices[faces[:,0],None,:]+(vertices[faces[:,1]]-vertices[faces[:,0]])[:,None,:]*uv[None,:,0,None]+(vertices[faces[:,2]]-vertices[faces[:,0]])[:,None,:]*uv[None,:,1,None]
        heights=interp(xy.reshape(-1,2)).reshape(len(faces),10)
        coeff.append(np.linalg.solve(basis(uv),heights.T).T)
    coefficients=np.stack(coeff,axis=1).astype('<f4')
    gradient=interp.grad[:,0,:]
    packed_vertices=np.column_stack([vertices[:,0]/n*extent-extent/2,extent/2-vertices[:,1]/n*extent,interp.values[:,0],gradient[:,0]*n/extent,-gradient[:,1]*n/extent]).astype('<f4')
    raw=struct.pack('<4sII',b'YCT1',len(vertices),len(faces))+packed_vertices.tobytes()+faces.astype('<u4').tobytes()+coefficients.tobytes()
    return zlib.compress(raw,9)

def fit(grid,budget,extent=24000):
    n=128;sample=grid[::8,::8];yy,xx=np.indices(sample.shape);xy=np.column_stack([xx.ravel(),yy.ravel()]);z=sample.ravel()
    chosen={0,n,(n+1)*n,(n+1)**2-1,int(np.argmax(z)),int(np.argmin(z))}
    ry,rx=np.gradient(sample,extent/n)
    previous=None
    for iteration in range(150):
        ids=np.array(sorted(chosen));interp=CloughTocher2DInterpolator(xy[ids],z[ids],tol=1e-8,maxiter=1000)
        prediction=interp(xy).reshape(sample.shape)
        py,px=np.gradient(prediction,extent/n)
        height_error=np.abs(prediction-sample)
        slope_error=np.sqrt((px-rx)**2+(py-ry)**2)
        score=height_error+60*slope_error
        body=pack_surface(interp,extent,n)
        report=dict(vertices=len(ids),triangles=len(interp.tri.simplices),bytes=len(body),maxError=float(height_error.max()),rmse=float(np.sqrt(np.mean(height_error**2))),slopeRMSE=float(np.sqrt(np.mean(slope_error**2))),budget=budget)
        if len(body)>budget and previous is not None:return previous
        previous=(body,report,interp)
        if report['maxError']<1 or len(chosen)>4000:return previous
        simplexes=interp.tri.find_simplex(xy);order=np.argsort(-score.ravel());order=order[score.ravel()[order]>0]
        _,first=np.unique(simplexes[order],return_index=True)
        candidates=[int(i) for i in order[np.sort(first)] if int(i) not in chosen]
        if not candidates:return previous
        chosen.update(candidates[:max(4,min(64,len(chosen)//4))])
    return previous

def generate(database=ROOT/'build/fixed-mesh-preview.sqlite3'):
    with sqlite3.connect(database) as db:
        db.execute('CREATE TABLE IF NOT EXISTS curve_blobs(region TEXT,variant INTEGER,data BLOB,PRIMARY KEY(region,variant))')
        manifest=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
        for region,meta in manifest['regions'].items():
            grid=np.frombuffer(zlib.decompress(db.execute('SELECT data FROM blobs WHERE region=? AND kind="height" AND variant=?',(region,meta['gridSize'])).fetchone()[0]),'<f4').reshape(meta['gridSize'],meta['gridSize'])
            reports=[]
            for tolerance in (1000,500,250,100,50):
                original=next(m for m in meta['meshes'] if m['tolerance']==tolerance)
                budget=max(4096,original['bytes'])
                body,report,interp=fit(grid,budget,meta['extent'])
                # 学習格子に含まれない地点も含む257四方で照合する。
                y,x=np.indices((257,257));prediction=interp(np.column_stack([x.ravel()/2,y.ravel()/2])).reshape(257,257)
                errors=prediction-grid[::4,::4]
                report.update(tolerance=tolerance,maxError=float(np.abs(errors).max()),rmse=float(np.sqrt(np.mean(errors**2))),linearBytes=original['bytes'],model='C1曲面・容量基準',validationSize=257)
                db.execute('INSERT OR REPLACE INTO curve_blobs VALUES(?,?,?)',(region,tolerance,body));reports.append(report)
                meta['curveMeshes']=sorted(reports,key=lambda m:m['tolerance'])
                db.execute('UPDATE metadata SET value=? WHERE key="manifest"',(json.dumps(manifest,ensure_ascii=False),));db.commit();print(region,report,flush=True)
if __name__=='__main__':generate()
