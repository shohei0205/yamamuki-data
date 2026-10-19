"""元DEMに曲面を当てはめ、頂点ごとの2成分の傾きを保存する。"""
import json,sqlite3,struct,zlib,hashlib
import numpy as np
from scipy import sparse
from scipy.sparse.linalg import lsmr
from scipy.ndimage import gaussian_filter,map_coordinates
from progressive_tin_compare import ROOT,REGIONS,index_blob
from fixed_mesh_preview import source_grid
from tin_vertex_optimize import raster

SCALE=8.
def design(points,faces,owner,w):
    ids=faces[owner];tri=points[ids];abc=np.prod(w,axis=1)
    base=(w**3*tri[:,:,2]).sum(axis=1)-abc*tri[:,:,2].sum(axis=1)
    rows=[];cols=[];values=[]
    for a,b in [(0,1),(1,0),(1,2),(2,1),(2,0),(0,2)]:
        coefficient=3*w[:,a]**2*w[:,b]+1.5*abc
        base+=coefficient*tri[:,a,2]
        for axis in [0,1]:
            rows.append(np.arange(len(w)));cols.append(ids[:,a]*2+axis)
            values.append(coefficient*(tri[:,b,axis]-tri[:,a,axis])/3)
    return base,sparse.csr_matrix((np.concatenate(values),(np.concatenate(rows),np.concatenate(cols))),shape=(len(w),len(points)*2))

def generate():
    report={}
    with sqlite3.connect(ROOT/'build/curve-hints.sqlite3') as out,sqlite3.connect((ROOT/'build/terrain-lod.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as src:
        out.execute('CREATE TABLE IF NOT EXISTS hints(region TEXT,level INTEGER,data BLOB,PRIMARY KEY(region,level))')
        for region,(_,lat,lon) in REGIONS.items():
            grid,_=source_grid(src,lat,lon,extent=16000,size=685);grid=np.rint(grid*4)/4
            dy,dx=np.gradient(gaussian_filter(grid.astype(float),1),16000/684)
            report[region]=[]
            for level in range(6):
                mesh=json.loads((ROOT/f"build/progressive-{region}{'-reference' if level==3 else '-coarse' if level>=4 else ''}.json").read_text(encoding='utf-8'))
                stage=mesh['stages'][0 if level==3 else level-4 if level>=4 else level];p=np.asarray(mesh['vertices'][:stage['vertexCount']],float);faces=np.array([f['vertices'] for f in stage['faces']])
                owner,w=raster(p,faces,685)
                p[:,:2]*=16000/684
                base,A=design(p,faces,owner,w)
                boundary=np.zeros((685,685),bool);boundary[[0,-1],:]=True;boundary[:,[0,-1]]=True
                linear=(p[faces[owner],2]*w).sum(axis=1)
                base[boundary.ravel()]=linear[boundary.ravel()]
                A=sparse.diags((~boundary.ravel()).astype(float))@A
                y=p[:,1]*684/16000;x=p[:,0]*684/16000
                initial=np.column_stack([map_coordinates(dx,[y,x],order=1),map_coordinates(dy,[y,x],order=1)]).ravel()
                # 5 m相当の弱い正則化で、未拘束方向の傾きが暴れないようにする。
                delta=lsmr(A,grid.ravel()-base-A@initial,damp=5.,atol=1e-6,btol=1e-6,maxiter=160)[0]
                slope=np.clip(initial+delta,-SCALE,SCALE)
                encoded=np.rint(slope/SCALE*32767).astype('<i2');decoded=encoded.astype(float)*SCALE/32767
                body=zlib.compress(struct.pack('<4sIf',b'YCH1',len(p),SCALE)+hashlib.sha256(index_blob(stage)).digest()+encoded.tobytes(),9)
                out.execute('INSERT OR REPLACE INTO hints VALUES(?,?,?)',(region,level,body))
                linear=(p[faces[owner],2]*w).sum(axis=1);fitted=base+A@decoded;raw=base+A@initial
                metrics=lambda z:dict(rmse=float(np.sqrt(np.mean((z-grid.ravel())**2))),maxError=float(np.abs(z-grid.ravel()).max()))
                entry=dict(vertices=len(p),bytes=len(body),linear=metrics(linear),demSlopes=metrics(raw),fitted=metrics(fitted))
                report[region].append(entry);out.commit();print(region,level,json.dumps(entry),flush=True)
    (ROOT/'build/curve-hints-report.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
if __name__=='__main__':generate()
