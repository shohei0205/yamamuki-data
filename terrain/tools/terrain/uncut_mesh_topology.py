"""保存境界で増えた点を平面形状を保って除き、元の接続を持つ余白付きタイルを作る。"""
import json, sqlite3, struct, zlib
from collections import defaultdict, deque
from pathlib import Path
import numpy as np
from national_independent_mesh import ROOT


def restore(points, faces, candidates, tolerance=.01):
    p=np.asarray(points,dtype=float); f={i:list(map(int,v)) for i,v in enumerate(faces)}
    incident=[set() for _ in p]
    for i,tri in f.items():
        for v in tri:incident[v].add(i)
    pending=deque(map(int,candidates)); active=set(map(int,candidates)); removed=0
    while pending:
        v=pending.popleft()
        if v not in active or not incident[v]:continue
        ids=list(incident[v]); near=set(j for i in ids for j in f[i]);near.discard(v)
        boundary_near=[j for j in near if len(incident[v]&incident[j])==1]
        for target in sorted(near,key=lambda j:float(np.linalg.norm(p[j,:2]-p[v,:2]))):
            if boundary_near:
                if target not in boundary_near or len(boundary_near)!=2:continue
                a,b=p[boundary_near,:2];delta=b-a
                if abs(np.linalg.det(np.column_stack([delta,p[v,:2]-a])))>1e-7:continue
            shared=incident[v]&incident[target]
            target_near={j for i in incident[target] for j in f[i]};target_near.discard(target)
            opposite={j for i in shared for j in f[i] if j not in (v,target)}
            if near&target_near!=opposite or len(shared) not in (1,2):continue
            changes={};ok=True
            for i in ids:
                if i in shared:continue
                before=p[f[i]];tri=[target if j==v else j for j in f[i]];after=p[tri]
                n=np.cross(before[1]-before[0],before[2]-before[0])
                new=np.cross(after[1]-after[0],after[2]-after[0])
                if new[2]<=1e-8 or abs(n[2])<1e-12 or abs(np.dot(n,p[target]-before[0])/n[2])>tolerance:
                    ok=False;break
                changes[i]=tri
            if not ok:continue
            for i in ids:
                for j in f[i]:incident[j].discard(i)
                del f[i]
            for i,tri in changes.items():
                f[i]=tri
                for j in tri:incident[j].add(i)
            active.remove(v);removed+=1
            for j in near&active:pending.append(j)
            break
    return np.asarray(list(f.values()),dtype=np.uint32),removed


def vertex_normals(p,f):
    q=p[f];cross=np.cross(q[:,1]-q[:,0],q[:,2]-q[:,0]);sums=np.zeros_like(p,dtype=float)
    for k in range(3):np.add.at(sums,f[:,k],cross)
    length=np.linalg.norm(sums,axis=1);return sums/np.maximum(length[:,None],1e-20)


def tile_topology(p,f,width=8000.,rings=5,ranks=None):
    incident=[[] for _ in p]
    for i,tri in enumerate(f):
        for v in tri:incident[v].append(i)
    owners=defaultdict(list)
    q=p[f,:2];lo=np.floor(q.min(axis=1)/width).astype(int);hi=np.floor(q.max(axis=1)/width).astype(int)
    for i,(a,b) in enumerate(zip(lo,hi)):
        for x in range(a[0],b[0]+1):
            for y in range(a[1],b[1]+1):owners[x,y].append(i)
    normals=vertex_normals(p,f)
    for (x,y),core in sorted(owners.items()):
        # 曲面で境界を越えて移動する面も描画候補に含める。
        core=sorted(set(core)|{i for v in f[core].ravel() for i in incident[v]})
        selected=set(core);front={int(v) for v in f[core].ravel()};seen=set(front)
        for _ in range(rings):
            added={i for v in front for i in incident[v]}-selected;selected|=added
            front={int(v) for v in f[list(added)].ravel()}-seen if added else set();seen|=front
        ids=core+sorted(selected-set(core));tri=f[ids];used,idx=np.unique(tri,return_inverse=True)
        points=p[used].copy();points[:,:2]-=[x*width,y*width]
        body=struct.pack('<4sIIII',b'YNU2',257,len(used),len(tri)*3,len(core)*3)+points.astype('<f4').tobytes()+idx.astype('<u4').tobytes()+normals[used].astype('<f4').tobytes()+(np.asarray(ranks)[used] if ranks is not None else np.full(len(used),-1)).astype('<f4').tobytes()
        yield (x,y),zlib.compress(body,9),len(core)


def update_metrics(db,manifest):
    for area in manifest['reliefRegions']:
        level=area['levels'][0];size=0;vertices=0
        for key in level['keys']:
            body=db.execute('SELECT data FROM topology WHERE level=? AND x=? AND y=?',key).fetchone()[0]
            raw=zlib.decompress(body);nv=struct.unpack_from('<I',raw,8)[0];size+=len(body);vertices+=nv
        metrics=level['priorityMetrics'];metrics.setdefault('clippedGeometryBytes',metrics['geometryBytes']);metrics.setdefault('clippedVertices',metrics['vertices'])
        metrics['geometryBytes']=size;metrics['vertices']=vertices
    db.execute('UPDATE metadata SET value=? WHERE key="manifest"',(json.dumps(manifest),))


def rebuild(path):
    with sqlite3.connect(path) as db:
        manifest=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
        if manifest['sharedAllocation'].get('topology')=='uncut-halo-v3':
            update_metrics(db,manifest);return
        db.execute('CREATE TABLE IF NOT EXISTS topology(level,x,y,data,PRIMARY KEY(level,x,y))')
        db.execute('DELETE FROM topology');reports=[]
        for area in manifest['reliefRegions']:
            points=[];faces=[];lookup={};ranks=[];keys=area['levels'][0]['keys'];ox=min(k[1] for k in keys);oy=min(k[2] for k in keys)
            for l,x,y in keys:
                raw=zlib.decompress(db.execute('SELECT geometry FROM tiles WHERE level=? AND x=? AND y=?',(l,x,y)).fetchone()[0]);_,n,_,nv,ni=struct.unpack_from('<4sHHII',raw)
                p=np.frombuffer(raw,'<f4',nv*3,16).reshape(-1,3).astype(float);p[:,:2]+=[(x-ox)*8000,(y-oy)*8000]
                f=np.frombuffer(raw,'<u2',ni,16+nv*12).reshape(-1,3);rank=np.frombuffer(raw,'<f4',nv,16+nv*12+ni*2);remap=[]
                for v,r in zip(p,rank):
                    key=tuple(np.rint(v*np.array([256,256,1024])).astype(int));idx=lookup.get(key)
                    if idx is None:idx=len(points);lookup[key]=idx;points.append(v);ranks.append(float(r))
                    else:ranks[idx]=max(ranks[idx],float(r))
                    remap.append(idx)
                faces.extend(np.asarray(remap)[f])
            p=np.asarray(points);f=np.asarray(faces);ranks=np.asarray(ranks)
            on_source_grid=np.max(np.abs(p[:,:2]/31.25-np.rint(p[:,:2]/31.25)),axis=1)<1e-6
            candidates=np.flatnonzero((ranks<0)&~on_source_grid)
            restored,removed=restore(p,f,candidates)
            def surface_area(indices):
                q=p[indices,:2];return np.linalg.det(np.stack([q[:,1]-q[:,0],q[:,2]-q[:,0]],axis=2)).sum()/2
            assert abs(surface_area(restored)-surface_area(f))<.05, '接続復元で面積が変化しました'
            records={tuple(k[1:]):k for k in keys};written=0
            for (x,y),body,count in tile_topology(p,restored,ranks=ranks):
                key=records.get((x+ox,y+oy))
                if key is None:continue
                db.execute('INSERT INTO topology VALUES(?,?,?,?)',(*key,body));written+=1
            assert written==len(keys)
            reports.append(dict(region=area['id'],artificialCandidates=len(candidates),removed=removed,facesBefore=len(f),facesAfter=len(restored),tiles=written))
            print(path.name, reports[-1],flush=True)
        manifest['sharedAllocation']['topology']='uncut-halo-v3';manifest['sharedAllocation']['topologyReports']=reports
        update_metrics(db,manifest)


if __name__=='__main__':
    import argparse
    parser=argparse.ArgumentParser();parser.add_argument('path',type=Path);args=parser.parse_args();rebuild(args.path)
