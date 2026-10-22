"""宝篋山・筑波山の局所形状と斜面方向を、実際の三角形接続で比較する。"""
import json
import numpy as np
from national_independent_mesh import ROOT,project
from tin_vertex_optimize import raster,predicted

PEAKS=[('hokyo',140.13,36.1675),('tsukuba',140.1069,36.2254)]


def local_metrics(p,f,ids,base,grid,c):
    lo=np.rint(c).astype(int)-64;size=129;hi=lo+size-1
    tri=p[f,:2];keep=np.all(tri.max(axis=1)>=lo,axis=1)&np.all(tri.min(axis=1)<=hi,axis=1)
    faces=f[keep];used,remap=np.unique(faces,return_inverse=True);points=p[used].copy();points[:,:2]-=lo;faces=remap.reshape(-1,3)
    owner,w=raster(points,faces,size);height=predicted(points,faces,owner,w).reshape(size,size)
    yy,xx=np.mgrid[lo[1]:hi[1]+1,lo[0]:hi[0]+1];ref=grid[yy,xx]
    gx=(grid[yy,xx+2]-grid[yy,xx-2])/(4*31.25);gy=(grid[yy+2,xx]-grid[yy-2,xx])/(4*31.25)
    q=points[faces];u=q[:,1]-q[:,0];v=q[:,2]-q[:,0];cross=np.cross(u*np.array([31.25,31.25,1.]),v*np.array([31.25,31.25,1.]));cross/=np.linalg.norm(cross,axis=1)[:,None]
    normal=cross[owner].reshape(size,size,3);source=np.stack([-gx,-gy,np.ones_like(gx)],axis=-1);source/=np.linalg.norm(source,axis=-1)[...,None]
    degrees=np.degrees(np.arccos(np.clip((normal*source).sum(axis=-1),-1,1)))
    distance=np.hypot((xx-c[0])*31.25,(yy-c[1])*31.25);inside=distance<=1000
    vertex_distance=np.linalg.norm((p[:,:2]-c)*31.25,axis=1)
    return dict(vertices1km=int((vertex_distance<=1000).sum()),added1km=int(((vertex_distance<=1000)&(ids>=base)).sum()),heightRMSE=float(np.sqrt(np.mean((height[inside]-ref[inside])**2))),normalMeanDegrees=float(degrees[inside].mean()),normalP95Degrees=float(np.percentile(degrees[inside],95)))


def audit():
    meta=json.loads((ROOT/'build/terrain-quality-source-cache-v1/tsukuba.json').read_text())
    origin=np.array(meta['key'][1:])*128000+meta['offset']
    with np.load(ROOT/'build/terrain-quality-source-cache-v1/tsukuba.npz') as a:grid=a['grid']
    report={}
    for label,prefix in [('perceptual','terrain-error20-perceptual'),('shape','terrain-error20-shape'),('cut10','terrain-error20-cut10')]:
        folder=ROOT/f'build/{prefix}-tsukuba'
        if label=='cut10' and not (folder/'pruned.npz').exists():continue
        with np.load(folder/'base-quality.npz') as data:base=len(data['p'])
        with np.load(folder/'pruned.npz') as data:p=data['p'];f=data['f'];ids=data['ids']
        for name,lon,lat in PEAKS:
            center=(np.array(project(lon,lat))-origin)/31.25
            report.setdefault(name,{})[label]=local_metrics(p,f,ids,base,grid,center)
    (ROOT/'build/terrain-small-mountain-shape-comparison.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
    print(json.dumps(report,ensure_ascii=False),flush=True)
    return report

if __name__=='__main__':audit()
