"""既存TINの平面形状を変えず、小面積の独立BLOBへ分割する。"""
import json, sqlite3, struct, zlib
from pathlib import Path
import numpy as np
from vertex_normals import decode_oct
from terrain_levels import definitions,level_for_error
ROOT=Path(__file__).resolve().parents[2]
WIDTHS={1000:16000,500:16000,250:16000,100:8000,50:8000,30:4000,20:4000,10:2000,5:2000,2:1000}

def clip(poly,axis,edge,sign):
    out=[]
    for a,b in zip(poly,poly[1:]+poly[:1]):
        ia=sign*(a[axis]-edge)>=-1e-8;ib=sign*(b[axis]-edge)>=-1e-8
        if ia:out.append(a)
        if ia!=ib:
            f=(edge-a[axis])/(b[axis]-a[axis]);v=a+(b-a)*f;v[axis]=edge;out.append(v)
    return out

def generate(database=None):
    with sqlite3.connect(database or ROOT/'build/fixed-mesh-preview.sqlite3') as db:
        db.execute('CREATE TABLE IF NOT EXISTS adaptive_parts(region TEXT,tolerance INTEGER,x INTEGER,y INTEGER,data BLOB,PRIMARY KEY(region,tolerance,x,y))')
        manifest=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
        manifest['terrainLevels']=definitions()
        manifest['meshLevels']=definitions()
        manifest['levelNaming']='L{tileSideKm}_{quality}; mesh, heightmap, normalmap'
        for region,meta in manifest['regions'].items():
            records={}
            for tolerance,width in WIDTHS.items():
                n=int(np.ceil(meta['extent']/width))
                floor={100:512,250:128,500:64,1000:32}.get(tolerance,4)
                record=next(r for r in (meta['adaptiveMeshes'] if floor==4 else meta['adaptiveFloors'][str(floor)]) if r['tolerance']==tolerance)
                kind=record.get('blobKind','adaptive')
                raw=zlib.decompress(db.execute('SELECT data FROM blobs WHERE region=? AND kind=? AND variant=?',(region,kind,tolerance)).fetchone()[0]);nv,ni=struct.unpack_from('<II',raw)
                points=np.frombuffer(raw,'<f4',nv*3,8).reshape(-1,3).astype(float);faces=np.frombuffer(raw,'<u4',ni,8+nv*12).reshape(-1,3)
                packet=zlib.decompress(db.execute('SELECT data FROM blobs WHERE region=? AND kind=? AND variant=?',(region,'vertex_normal_'+kind,tolerance)).fetchone()[0]);normals=decode_oct(np.frombuffer(packet,np.uint8,nv*2,40).reshape(-1,2))
                triangles=points[faces];lo=triangles[:,:,:2].min(axis=1);hi=triangles[:,:,:2].max(axis=1);parts=[]
                for y in range(n):
                    for x in range(n):
                        xmin=-meta['extent']/2+x*width;ymin=-meta['extent']/2+y*width;xmax=xmin+width;ymax=ymin+width
                        selected=np.flatnonzero((hi[:,0]>=xmin)&(lo[:,0]<=xmax)&(hi[:,1]>=ymin)&(lo[:,1]<=ymax))
                        inside=selected[(lo[selected,0]>=xmin)&(hi[selected,0]<=xmax)&(lo[selected,1]>=ymin)&(hi[selected,1]<=ymax)]
                        ids,inverse=np.unique(faces[inside].ravel(),return_inverse=True)
                        vertices=list(np.column_stack((points[ids],normals[ids])));indices=inverse.tolist();lookup={tuple(np.round(v[:3],5)):i for i,v in enumerate(vertices)}
                        for index in np.setdiff1d(selected,inside):
                            poly=[v.copy() for v in np.column_stack((points[faces[index]],normals[faces[index]]))]
                            for axis,edge,sign in [(0,xmin,1),(0,xmax,-1),(1,ymin,1),(1,ymax,-1)]:
                                if not poly:break
                                poly=clip(poly,axis,edge,sign)
                            for j in range(1,len(poly)-1):
                                tri=[poly[0],poly[j],poly[j+1]]
                                a=tri[1][:2]-tri[0][:2];b=tri[2][:2]-tri[0][:2]
                                if abs(a[0]*b[1]-a[1]*b[0])<1e-6:continue
                                for v in tri:
                                    key=tuple(np.round(v[:3],5));i=lookup.get(key)
                                    if i is None:i=len(vertices);lookup[key]=i;vertices.append(v)
                                    indices.append(i)
                        data=np.asarray(vertices);p=data[:,:3].astype('<f4');normal=data[:,3:];normal/=np.linalg.norm(normal,axis=1,keepdims=True);normal=normal.astype('<f4');idx=np.asarray(indices,dtype='<u4')
                        # 境界で極細になった面はFloat32への丸めで潰れる場合がある。
                        f=idx.reshape(-1,3);a=p[f[:,1],:2].astype(float)-p[f[:,0],:2];b=p[f[:,2],:2].astype(float)-p[f[:,0],:2]
                        idx=f[a[:,0]*b[:,1]-a[:,1]*b[:,0]>0].ravel()
                        body=zlib.compress(struct.pack('<II',len(p),len(idx))+p.tobytes()+idx.tobytes()+normal.tobytes(),9)
                        db.execute('INSERT OR REPLACE INTO adaptive_parts VALUES(?,?,?,?,?)',(region,tolerance,x,y,body))
                        parts.append(dict(x=x,y=y,validWidthMeters=min(width,meta['extent']-x*width),validHeightMeters=min(width,meta['extent']-y*width),vertices=len(p),triangles=len(idx)//3,bytes=len(body)))
                records[str(tolerance)]=dict(level=level_for_error(tolerance),divisions=n,widthMeters=width,areaKm2=(width/1000)**2,parts=parts,bytes=sum(p['bytes'] for p in parts))
                print(region,tolerance,n,'parts',len(parts),'bytes',records[str(tolerance)]['bytes'],flush=True)
            meta['adaptiveSpatial']=records
            db.execute('UPDATE metadata SET value=? WHERE key="manifest"',(json.dumps(manifest,ensure_ascii=False),));db.commit()
if __name__=='__main__':generate()
