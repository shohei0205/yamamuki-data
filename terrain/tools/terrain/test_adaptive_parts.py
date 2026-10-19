"""小面積メッシュの範囲・面積・隣接境界・法線を検証する。"""
import json,sqlite3,struct,zlib
from pathlib import Path
import numpy as np
from adaptive_mesh_parts import ROOT,WIDTHS

def verify(database):
    reports=[]
    with sqlite3.connect(database) as db:
        m=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
        for region,meta in m['regions'].items():
            assert meta['extent']==32000
            for tolerance,width in WIDTHS.items():
                record=meta['adaptiveSpatial'][str(tolerance)];n=32000//width;assert record['divisions']==n and record['areaKm2']==(width/1000)**2
                edges={};total=0
                for y in range(n):
                    for x in range(n):
                        body=zlib.decompress(db.execute('SELECT data FROM adaptive_parts WHERE region=? AND tolerance=? AND x=? AND y=?',(region,tolerance,x,y)).fetchone()[0]);nv,ni=struct.unpack_from('<II',body)
                        assert len(body)==8+nv*24+ni*4
                        p=np.frombuffer(body,'<f4',nv*3,8).reshape(-1,3);i=np.frombuffer(body,'<u4',ni,8+nv*12).reshape(-1,3);normals=np.frombuffer(body,'<f4',nv*3,8+nv*12+ni*4).reshape(-1,3)
                        assert i.max()<nv and np.isfinite(p).all() and np.max(np.abs(np.linalg.norm(normals,axis=1)-1))<1e-5
                        bounds=[-16000+x*width,-16000+(x+1)*width,-16000+y*width,-16000+(y+1)*width]
                        for axis in range(2):assert p[:,axis].min()>=bounds[axis*2]-.002 and p[:,axis].max()<=bounds[axis*2+1]+.002
                        a=p[i[:,1],:2].astype(float)-p[i[:,0],:2];b=p[i[:,2],:2].astype(float)-p[i[:,0],:2];area=(a[:,0]*b[:,1]-a[:,1]*b[:,0])/2
                        assert area.min()>0 and abs(area.sum()-width**2)<width**2*2e-6
                        total+=area.sum()
                        def edge(axis,boundary):
                            used=p[np.unique(i)];points=used[np.abs(used[:,axis]-boundary)<.001];order=np.lexsort((points[:,2],points[:,1-axis]));return points[order][:,[1-axis,2]]
                        left,right,bottom,top=edge(0,bounds[0]),edge(0,bounds[1]),edge(1,bounds[2]),edge(1,bounds[3])
                        def same_profile(a,b):
                            # 同一直線上の余分な境界頂点があっても、高さの連続性を全節点で確かめる。
                            coords=np.unique(np.concatenate((a[:,0],b[:,0])))
                            np.testing.assert_allclose(np.interp(coords,a[:,0],a[:,1]),np.interp(coords,b[:,0],b[:,1]),atol=.005,rtol=0)
                        if x:same_profile(edges[x-1,y][0],left)
                        if y:same_profile(edges[x,y-1][1],bottom)
                        edges[x,y]=(right,top)
                assert abs(total-32000**2)<32000**2*2e-6
                reports.append(dict(region=region,tolerance=tolerance,tiles=n*n,widthMeters=width,areaKm2=width**2/1e6));print(region,tolerance,n*n,'OK',flush=True)
    (ROOT/'build/adaptive-spatial-validation.json').write_text(json.dumps(reports,indent=2),encoding='utf-8')
if __name__=='__main__':verify(ROOT/'build/fixed-mesh-preview-32km.sqlite3')
