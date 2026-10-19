"""標高誤差の上限を保ち、三角形の辺を交換して傾き誤差を減らす。"""
import hashlib,json,sqlite3,struct,zlib
from pathlib import Path
import numpy as np
from scipy.ndimage import gaussian_filter
ROOT=Path(__file__).resolve().parents[2]

def triangle_values(xy,z,points):
    a,b,c=xy;den=(b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])
    if abs(den)<1e-9:return None
    u=((points[:,0]-a[0])*(c[1]-a[1])-(points[:,1]-a[1])*(c[0]-a[0]))/den
    v=((b[0]-a[0])*(points[:,1]-a[1])-(b[1]-a[1])*(points[:,0]-a[0]))/den
    inside=(u>=-1e-7)&(v>=-1e-7)&(u+v<=1.0000001)
    prediction=z[0]+u*(z[1]-z[0])+v*(z[2]-z[0])
    normal=np.cross(np.column_stack([xy,z])[1]-np.r_[a,z[0]],np.column_stack([xy,z])[2]-np.r_[a,z[0]])
    if normal[2]<0:normal=-normal
    return inside,prediction,normal

def optimize(grid,vertices,faces,tolerance,extent=24000,passes=2):
    size=len(grid);n=size-1;step=extent/n
    xy=np.column_stack([(vertices[:,0]/extent+.5)*n,(.5-vertices[:,1]/extent)*n]);z=vertices[:,2]
    dy,dx=np.gradient(gaussian_filter(grid,1),step);reference=np.stack([-dx,-dy,np.ones_like(dx)],axis=-1);reference/=np.linalg.norm(reference,axis=2,keepdims=True)
    def bounds(ids):
        coords=xy[ids];lo=np.maximum(0,np.floor(coords.min(axis=0)).astype(int));hi=np.minimum(n,np.ceil(coords.max(axis=0)).astype(int));y,x=np.indices((hi[1]-lo[1]+1,hi[0]-lo[0]+1));return np.column_stack([(x+lo[0]).ravel(),(y+lo[1]).ravel()])
    def raster(triangles,points):
        pred=np.full(len(points),np.nan);angles=np.full(len(points),np.nan)
        for tri in triangles:
            result=triangle_values(xy[tri],z[tri],points)
            if result is None:continue
            mask,value,normal=result;normal[:2]/=step;normal/=np.linalg.norm(normal)
            pred[mask]=value[mask];angles[mask]=np.arccos(np.clip(reference[points[mask,1],points[mask,0]]@normal,-1,1))
        return pred,angles
    prediction=np.full(grid.shape,np.nan);angles=np.full(grid.shape,np.nan)
    for tri in faces:
        pts=bounds(tri);pred,ang=raster([tri],pts);valid=np.isfinite(pred);prediction[pts[valid,1],pts[valid,0]]=pred[valid];angles[pts[valid,1],pts[valid,0]]=ang[valid]
    if not np.isfinite(prediction).all():raise ValueError('元メッシュに未被覆の格子があります')
    original_errors=prediction-grid;baseline=dict(maxError=float(np.abs(original_errors).max()),rmse=float(np.sqrt(np.mean(original_errors**2))),meanAngle=float(np.degrees(angles).mean()))
    flips=0
    for iteration in range(passes):
        edges={}
        for i,tri in enumerate(faces):
            for j in range(3):edges.setdefault(tuple(sorted((int(tri[j]),int(tri[(j+1)%3])))),[]).append(i)
        touched=set()
        for (a,b),adj in edges.items():
            if len(adj)!=2 or any(i in touched for i in adj):continue
            f,g=adj;c=next(int(i) for i in faces[f] if i not in (a,b));d=next(int(i) for i in faces[g] if i not in (a,b))
            if c==d or tuple(sorted((c,d))) in edges:continue
            cross=lambda u,v:float(u[0]*v[1]-u[1]*v[0])
            if cross(xy[b]-xy[a],xy[c]-xy[a])*cross(xy[b]-xy[a],xy[d]-xy[a])>=0:continue
            if cross(xy[d]-xy[c],xy[a]-xy[c])*cross(xy[d]-xy[c],xy[b]-xy[c])>=0:continue
            coords=xy[[a,b,c,d]]
            if np.prod(np.ceil(coords.max(axis=0))-np.floor(coords.min(axis=0))+1)>65536:continue
            pts=bounds([a,b,c,d]);old,old_angle=raster(faces[[f,g]],pts);new_faces=np.array([[c,d,a],[d,c,b]],dtype=np.uint32);new,new_angle=raster(new_faces,pts);valid=np.isfinite(old)&np.isfinite(new)
            if not valid.any():continue
            truth=grid[pts[valid,1],pts[valid,0]];old_error=old[valid]-truth;new_error=new[valid]-truth
            if np.abs(new_error).max()>tolerance+.01:continue
            if new_angle[valid].mean()>=old_angle[valid].mean()*.99:continue
            old_score=np.mean((old_error/tolerance)**2+.5*(old_angle[valid]/.17)**2)
            new_score=np.mean((new_error/tolerance)**2+.5*(new_angle[valid]/.17)**2)
            if new_score>=old_score*.99:continue
            faces[f],faces[g]=new_faces;prediction[pts[valid,1],pts[valid,0]]=new[valid];angles[pts[valid,1],pts[valid,0]]=new_angle[valid];touched.update(adj);flips+=1
    errors=prediction-grid
    return faces,dict(maxError=float(np.abs(errors).max()),rmse=float(np.sqrt(np.mean(errors**2))),slopeMeanDegrees=float(np.degrees(angles).mean()),slopeP95Degrees=float(np.percentile(np.degrees(angles),95)),baselineMaxError=baseline['maxError'],baselineRMSE=baseline['rmse'],baselineSlopeMeanDegrees=baseline['meanAngle'],edgeFlips=flips)

def generate(database=ROOT/'build/fixed-mesh-preview.sqlite3'):
    with sqlite3.connect(database) as db:
        db.execute('CREATE TABLE IF NOT EXISTS slope_blobs(region TEXT,variant INTEGER,data BLOB,PRIMARY KEY(region,variant))')
        manifest=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
        for region,meta in manifest['regions'].items():
            size=meta['gridSize'];grid=np.frombuffer(zlib.decompress(db.execute('SELECT data FROM blobs WHERE region=? AND kind="height" AND variant=?',(region,size)).fetchone()[0]),'<f4').reshape(size,size);reports=[]
            for tolerance in (1000,500,250,100,50):
                raw=zlib.decompress(db.execute('SELECT data FROM blobs WHERE region=? AND kind="mesh" AND variant=?',(region,tolerance)).fetchone()[0]);nv,ni=struct.unpack_from('<II',raw);vertices=np.frombuffer(raw,'<f4',nv*3,8).reshape(-1,3);faces=np.frombuffer(raw,'<u4',ni,8+nv*12).reshape(-1,3).copy()
                faces,report=optimize(grid,vertices,faces,tolerance,meta['extent'])
                result=raw[:8+nv*12]+faces.astype('<u4').tobytes();body=zlib.compress(result,9)
                report.update(tolerance=tolerance,vertices=nv,triangles=len(faces),bytes=len(body),linearBytes=len(zlib.compress(raw,9)))
                db.execute('INSERT OR REPLACE INTO slope_blobs VALUES(?,?,?)',(region,tolerance,body))
                normal=db.execute('SELECT data FROM blobs WHERE region=? AND kind="vertex_normal" AND variant=?',(region,tolerance)).fetchone()
                if normal:
                    packet=zlib.decompress(normal[0]);packet=packet[:8]+hashlib.sha256(result).digest()+packet[40:];db.execute('INSERT OR REPLACE INTO blobs VALUES(?,?,?,?)',(region,'vertex_normal_slope',tolerance,zlib.compress(packet,9)));report['vertexNormalBytes']=len(zlib.compress(packet,9))
                reports.append(report);meta['slopeMeshes']=sorted(reports,key=lambda m:m['tolerance']);db.execute('UPDATE metadata SET value=? WHERE key="manifest"',(json.dumps(manifest,ensure_ascii=False),));db.commit();print(region,report,flush=True)
if __name__=='__main__':generate()
