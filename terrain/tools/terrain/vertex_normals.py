"""従来メッシュに元DEM由来の局所平均法線を2バイト/頂点で追加する。"""
import hashlib,json,sqlite3,struct,zlib
from pathlib import Path
import numpy as np
from scipy.ndimage import gaussian_filter,map_coordinates
from fixed_mesh_preview import source_grid
ROOT=Path(__file__).resolve().parents[2]

def encode_oct(normals):
    p=normals[:,:2]/np.abs(normals).sum(axis=1,keepdims=True)
    low=normals[:,2]<0
    p[low]=(1-np.abs(p[low,::-1]))*np.where(p[low]>=0,1,-1)
    return np.rint(np.clip(p*.5+.5,0,1)*255).astype(np.uint8)

def decode_oct(values):
    xy=values.astype(np.float64)/127.5-1
    z=1-np.abs(xy).sum(axis=1)
    low=z<0
    xy[low]=(1-np.abs(xy[low,::-1]))*np.where(xy[low]>=0,1,-1)
    n=np.column_stack([xy,z]);return n/np.linalg.norm(n,axis=1,keepdims=True)

def generate(database=ROOT/'build/fixed-mesh-preview.sqlite3',source=ROOT/'build/terrain-lod.sqlite3'):
    with sqlite3.connect(database) as db,sqlite3.connect(source.resolve().as_uri()+'?mode=ro',uri=True) as src:
        manifest=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
        for region,meta in manifest['regions'].items():
            _,reference=source_grid(src,meta['lat'],meta['lon'],meta['extent'])
            data,_,_,left,top,width,height=reference
            finite=np.isfinite(data);clean=np.where(finite,data,0)
            records=[]
            for mesh in meta['meshes']:
                raw=zlib.decompress(db.execute('SELECT data FROM blobs WHERE region=? AND kind="mesh" AND variant=?',(region,mesh['tolerance'])).fetchone()[0])
                nv,ni=struct.unpack_from('<II',raw);points=np.frombuffer(raw,'<f4',nv*3,8).reshape(-1,3)
                sigma=max(.5,width/(np.sqrt(nv)*4))
                averaged=gaussian_filter(clean,sigma,mode='nearest')/np.maximum(gaussian_filter(finite.astype(np.float32),sigma,mode='nearest'),1e-8)
                dy,dx=np.gradient(averaged,meta['extent']/width)
                x=(points[:,0]/meta['extent']+.5)*width+left;y=(.5-points[:,1]/meta['extent'])*height+top
                normals=np.column_stack([-map_coordinates(dx,[y,x],order=1,mode='nearest'),map_coordinates(dy,[y,x],order=1,mode='nearest'),np.ones(nv)])
                normals/=np.linalg.norm(normals,axis=1,keepdims=True)
                octets=encode_oct(normals);restored=decode_oct(octets)
                angle=np.degrees(np.arccos(np.clip((normals*restored).sum(axis=1),-1,1)))
                body=zlib.compress(struct.pack('<4sI',b'YVN1',nv)+hashlib.sha256(raw).digest()+octets.tobytes(),9)
                db.execute('INSERT OR REPLACE INTO blobs VALUES(?,?,?,?)',(region,'vertex_normal',mesh['tolerance'],body))
                report=dict(tolerance=mesh['tolerance'],vertices=nv,bytes=len(body),rawNormalBytes=nv*2,maxAngle=float(angle.max()),meanAngle=float(angle.mean()),sigmaMeters=float(sigma*meta['extent']/width),meshBytes=mesh['bytes'])
                records.append(report);print(region,report,flush=True)
            meta['vertexNormals']=records
            db.execute('UPDATE metadata SET value=? WHERE key="manifest"',(json.dumps(manifest,ensure_ascii=False),));db.commit()
if __name__=='__main__':generate()
