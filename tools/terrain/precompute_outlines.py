"""全国の輪郭を事前に平滑化し、動いた頂点の差分だけ保存する。"""
import argparse
from contextlib import closing
import gzip
import json
from pathlib import Path
import sqlite3
import time
import numpy as np
from scipy.spatial import cKDTree
from overlay_region import Surface
from polygon_dem import decode_mesh
from polygon_region import smooth_outline


def fingerprint(path):
    stat=path.stat()
    return {'bytes':stat.st_size,'modifiedNs':stat.st_mtime_ns}


def build(source,output,levels=(4,6,8,10)):
    temporary=output.with_suffix('.part.sqlite3')
    if temporary.exists():raise ValueError('未完成の出力があります')
    signature=fingerprint(source)
    reports=[]
    with closing(sqlite3.connect(source.resolve().as_uri()+'?mode=ro',uri=True)) as src,closing(sqlite3.connect(temporary)) as dest:
        dest.execute('CREATE TABLE tiles(z INTEGER,x INTEGER,y INTEGER,data BLOB,PRIMARY KEY(z,x,y)) WITHOUT ROWID')
        dest.execute('CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT)')
        for z in levels:
            start=time.perf_counter();vertices=[];faces=[];lookup={};mappings=[]
            for x,y,body in src.execute('SELECT x,y,data FROM tiles WHERE z=? ORDER BY x,y',(z,)):
                points,indices=decode_mesh(body);mapping=[]
                for vx,vy,height in points:
                    key=(x*256+int(vx),y*256+int(vy))
                    index=lookup.get(key)
                    if index is None:
                        index=len(vertices);lookup[key]=index;vertices.append([*key,float(height)])
                    elif abs(vertices[index][2]-height)>.001:raise ValueError('共有頂点の標高が一致しません')
                    mapping.append(index)
                mapping=np.asarray(mapping,dtype=np.int32)
                if indices.size:faces.extend(mapping[indices].ravel().tolist())
                mappings.append((x,y,mapping))
            if not faces:continue
            original=np.asarray(vertices,dtype=float);triangles=np.asarray(faces,dtype=np.int32).reshape(-1,3)
            smooth=smooth_outline(original,triangles.ravel(),1)
            changed=np.flatnonzero(np.linalg.norm(smooth[:,:2]-original[:,:2],axis=1)>1e-7)
            # 動いた外周の周囲だけを使って標高を補間する。
            selected=np.any(np.isin(triangles,changed),axis=1)
            heights=Surface(original,triangles[selected].ravel()).heights(smooth[changed,:2])
            missing=~np.isfinite(heights)
            if missing.any():heights[missing]=original[cKDTree(original[:,:2]).query(smooth[changed[missing],:2])[1],2]
            smooth[changed,2]=heights
            delta=smooth-original;changed_mask=np.zeros(len(original),dtype=bool);changed_mask[changed]=True
            saved=0;count=0;total=0
            for x,y,mapping in mappings:
                local=np.flatnonzero(changed_mask[mapping])
                if not len(local):continue
                rows=np.column_stack([local,delta[mapping[local]]]).astype('<f4')
                body=gzip.compress(rows.tobytes(),compresslevel=6,mtime=0)
                decoded=np.frombuffer(gzip.decompress(body),dtype='<f4').reshape(-1,4)
                np.testing.assert_allclose(decoded,rows,rtol=0,atol=0)
                dest.execute('INSERT INTO tiles VALUES (?,?,?,?)',(z,x,y,body));saved+=1;count+=len(local);total+=len(body)
            report={'zoom':z,'tilesWithChanges':saved,'vertexOffsets':count,'bytes':total,'seconds':time.perf_counter()-start}
            reports.append(report);dest.commit();print(json.dumps(report),flush=True)
        if fingerprint(source)!=signature:raise ValueError('元データが生成中に変更されました')
        meta={'schemaFormat':'yamamuki-terrain-outline-offsets','schemaVersion':1,'sourceFingerprint':signature,'levels':list(levels),'smoothingPasses':4,'maxMovementPixels':.6}
        dest.executemany('INSERT INTO metadata VALUES (?,?)',[(k,json.dumps(v)) for k,v in meta.items()]);dest.commit()
        assert dest.execute('PRAGMA quick_check').fetchone()[0]=='ok'
    temporary.replace(output)
    with output.open('rb') as src,gzip.open(str(output)+'.gz','wb') as dest:
        import shutil
        shutil.copyfileobj(src,dest)
    report={'levelReports':reports,'sqliteBytes':output.stat().st_size,'gzipBytes':Path(str(output)+'.gz').stat().st_size,**meta}
    output.with_suffix('.report.json').write_text(json.dumps(report,indent=2),encoding='utf-8')
    print(json.dumps(report),flush=True)
    return report

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=Path,default=Path('build/terrain-mesh.sqlite3'))
    parser.add_argument('--output',type=Path,default=Path('build/terrain-outlines.sqlite3'))
    args=parser.parse_args();build(args.source,args.output)
