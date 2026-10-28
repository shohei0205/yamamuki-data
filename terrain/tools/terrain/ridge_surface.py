"""稜線・谷線を保持する四角形主体の細分割曲面の地域比較。"""
import argparse
import base64
import functools
import gzip
import json
import sqlite3
import time
import urllib.request
import zlib
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlparse,parse_qs

import numpy as np
from scipy.ndimage import gaussian_filter,maximum_filter,minimum_filter
from scipy.spatial import Delaunay,cKDTree
from scipy.sparse import csr_matrix,eye
from scipy.sparse.linalg import lsqr
from shapely.geometry import LineString
from shapely.ops import unary_union
from shapely.strtree import STRtree
from lpf_curve_compare import Source,load_level,UNIT

SPAN=32000.
SETTINGS=[dict(id='coarse',label='粗い',flow=256,xy=750.,height=100.,extra=96),
          dict(id='medium',label='標準',flow=128,xy=375.,height=50.,extra=192),
          dict(id='fine',label='細かい',flow=64,xy=187.5,height=25.,extra=384),
          dict(id='half',label='三角形の約半分',flow=16,xy=60.,height=8.,extra=2048)]


def flow_lines(h,threshold,kind):
    """D8の下流（谷）と逆地形の下流（尾根）を幾何の候補にする。"""
    n=len(h);a=gaussian_filter(h,.7)*(1 if kind==2 else -1)
    best=np.zeros_like(a);destination=np.full(a.shape,-1,np.int64)
    ids=np.arange(a.size).reshape(a.shape)
    for dy,dx in [(-1,-1),(-1,0),(-1,1),(0,-1),(0,1),(1,-1),(1,0),(1,1)]:
        y0,y1=max(0,-dy),min(n,n-dy);x0,x1=max(0,-dx),min(n,n-dx)
        drop=(a[y0:y1,x0:x1]-a[y0+dy:y1+dy,x0+dx:x1+dx])/np.hypot(dx,dy)
        mask=drop>best[y0:y1,x0:x1]
        best[y0:y1,x0:x1][mask]=drop[mask]
        destination[y0:y1,x0:x1][mask]=ids[y0+dy:y1+dy,x0+dx:x1+dx][mask]
    dest=destination.ravel();acc=np.ones(a.size,np.int64)
    for i in np.argsort(a.ravel())[::-1]:
        if dest[i]>=0:acc[dest[i]]+=acc[i]
    active=(acc>=threshold)&((maximum_filter(h,9)-minimum_filter(h,9)).ravel()>30.)
    incoming=np.zeros(a.size,np.int32)
    for i in np.flatnonzero(active):
        if dest[i]>=0 and active[dest[i]]:incoming[dest[i]]+=1
    starts=np.flatnonzero(active&(incoming!=1));lines=[]
    for start in starts:
        path=[int(start)];current=int(start)
        while dest[current]>=0 and active[dest[current]]:
            current=int(dest[current]);path.append(current)
            if incoming[current]!=1:break
        if len(path)>1:
            q=np.array(path);xy=np.column_stack([q%n,q//n])*SPAN/(n-1)-SPAN/2
            lines.append(np.column_stack([xy,h.ravel()[q]]))
    return lines


def simplify(points,xy_tolerance,height_tolerance):
    keep={0,len(points)-1};stack=[(0,len(points)-1)]
    while stack:
        a,b=stack.pop()
        if b<=a+1:continue
        d=points[b,:2]-points[a,:2];den=d@d
        t=np.clip(((points[a+1:b,:2]-points[a,:2])@d)/max(den,1e-10),0,1)
        xy=points[a,:2]+t[:,None]*d
        score=np.maximum(np.linalg.norm(points[a+1:b,:2]-xy,axis=1)/xy_tolerance,
                         abs(points[a+1:b,2]-(points[a,2]+t*(points[b,2]-points[a,2])))/height_tolerance)
        i=int(np.argmax(score))+a+1
        if score[i-a-1]>1:keep.add(i);stack.extend([(a,i),(i,b)])
    return points[sorted(keep)]


def constrained_mesh(lines,peaks,source,center,settings):
    # 異なる骨格線の交点を同じ頂点にしてから制約辺を保護する。
    geoms=[LineString(line[:,:2]) for line,_ in lines];original=[(g,k) for g,(_,k) in zip(geoms,lines)]
    ridge_points=np.unique(np.vstack([line[:,:2] for line,k in lines if k==1]),axis=0) if any(k==1 for _,k in lines) else np.empty((0,2))
    if len(ridge_points):
        tree=cKDTree(ridge_points)
        for peak in peaks:
            distance,ids=tree.query(peak,k=min(12,len(ridge_points)))
            if np.min(distance)<.01:continue
            directions=[]
            for d,i in zip(np.atleast_1d(distance),np.atleast_1d(ids)):
                if d>1500:break
                direction=(ridge_points[i]-peak)/d
                if all(direction@v<.5 for v in directions):
                    g=LineString([peak,ridge_points[i]]);geoms.append(g);original.append((g,1));directions.append(direction)
                    if len(directions)==3:break
    border=np.array([[-16000,-16000],[16000,-16000],[16000,16000],[-16000,16000],[-16000,-16000]])
    for a,b in zip(border[:-1],border[1:]):
        xy=a+np.linspace(0,1,257)[:,None]*(b-a);height=source.sample(xy[:,0]+center[0],xy[:,1]+center[1])
        p=simplify(np.column_stack([xy,height]),1e9,settings['height']*.25)
        g=LineString(p[:,:2]);geoms.append(g);original.append((g,0))
    network=unary_union(geoms)
    original_tree=STRtree([g for g,k in original])
    coordinates=[];lookup={};edges={}
    def point(xy):
        key=tuple(np.round(xy,5))
        if key not in lookup:lookup[key]=len(coordinates);coordinates.append(key)
        return lookup[key]
    for line in getattr(network,'geoms',[network]):
        xy=np.array(line.coords)
        for a,b in zip(xy[:-1],xy[1:]):
            if np.linalg.norm(b-a)<.01:continue
            middle=LineString([a,b]).interpolate(.5,normalized=True)
            kind=original[int(np.min(original_tree.query_nearest(middle,all_matches=True)))][1]
            i,j=point(a),point(b);edges[tuple(sorted((i,j)))]=kind
    corners=set(point(p) for p in peaks)
    for xy in border[:-1]:corners.add(point(xy))
    # 長い外周も分割し、山腹だけに大きすぎる面が残らないようにする。
    for y in np.linspace(-16000,16000,9):
        for x in [-16000,16000]:point([x,y])
    for x in np.linspace(-16000,16000,9):
        for y in [-16000,16000]:point([x,y])
    axis=np.linspace(-15900,15900,settings.get('querySize',97));qx,qy=np.meshgrid(axis,axis);query=np.column_stack([qx.ravel(),qy.ravel()])
    target=source.sample(query[:,0]+center[0],query[:,1]+center[1])
    if settings.get('querySize'):print('骨格構築',len(coordinates),'制約',len(edges),flush=True)
    batch=settings.get('batch',16)
    for iteration in range((settings['extra']+batch-1)//batch):
        if settings.get('querySize') and iteration%8==0:print('山腹追加',iteration,'点',len(coordinates),flush=True)
        xy=np.array(coordinates);tri=Delaunay(xy);z=source.sample(xy[:,0]+center[0],xy[:,1]+center[1]);simplex=tri.find_simplex(query)
        t=tri.transform[simplex];bary=np.einsum('nij,nj->ni',t[:,:2],query-t[:,2]);bary=np.column_stack([bary,1-bary.sum(1)])
        error=abs(np.sum(z[tri.simplices[simplex]]*bary,axis=1)-target)
        if error.max()<=settings['height']:break
        chosen=[]
        for index in np.argsort(error)[::-1]:
            if error[index]<=settings['height']:break
            if all(np.linalg.norm(query[index]-query[j])>settings.get('spacing',500) for j in chosen):chosen.append(int(index))
            if len(chosen)==min(batch,settings['extra']-iteration*batch):break
        before=len(coordinates)
        for i in chosen:point(query[i])
        if len(coordinates)==before:break
    # 制約がDelaunayの辺になるまで中点を追加。線上の既存点でも分割する。
    for iteration in range(24):
        xy=np.array(coordinates);tri=Delaunay(xy)
        present={tuple(sorted((int(t[i]),int(t[(i+1)%3])))) for t in tri.simplices for i in range(3)}
        missing=[e for e in edges if e not in present]
        if not missing:break
        point_tree=cKDTree(xy)
        if settings.get('querySize'):print('制約辺確認',iteration,'点',len(xy),'未接続',len(missing),flush=True)
        for a,b in missing:
            kind=edges.pop((a,b));d=xy[b]-xy[a];den=d@d
            nearby=np.array(sorted(point_tree.query_ball_point((xy[a]+xy[b])*.5,np.sqrt(den)*.5+1e-4)),int)
            delta=xy[nearby]-xy[a];t=(delta[:,0]*d[0]+delta[:,1]*d[1])/den
            distance=np.linalg.norm(xy[nearby]-(xy[a]+np.clip(t,0,1)[:,None]*d),axis=1)
            valid=np.flatnonzero((distance<1e-4)&(t>1e-7)&(t<1-1e-7))
            if len(valid):middle=int(nearby[valid[np.argmin(abs(t[valid]-.5))]])
            else:middle=point((xy[a]+xy[b])/2)
            if middle in [a,b]:raise RuntimeError('制約辺を分割できません')
            edges[tuple(sorted((a,middle)))]=kind;edges[tuple(sorted((middle,b)))]=kind
    else:raise RuntimeError('骨格線の制約が収束しません')
    # 骨格線を横断せず、凸な四角形になる隣接三角形だけを結合する。
    adjacent={}
    for f,t in enumerate(tri.simplices):
        for i in range(3):adjacent.setdefault(tuple(sorted((int(t[i]),int(t[(i+1)%3])))),[]).append(f)
    candidates=[]
    for edge,faces in adjacent.items():
        if edge in edges or len(faces)!=2:continue
        ids=np.unique(tri.simplices[faces].ravel());p=xy[ids];ids=ids[np.argsort(np.arctan2(p[:,1]-p[:,1].mean(),p[:,0]-p[:,0].mean()))];p=xy[ids]
        d=np.roll(p,-1,axis=0)-p;cross=d[:,0]*np.roll(d,-1,axis=0)[:,1]-d[:,1]*np.roll(d,-1,axis=0)[:,0]
        if np.min(cross)<=1e-4:continue
        lengths=np.linalg.norm(d,axis=1);score=float(lengths.max()/max(1,lengths.min()))
        candidates.append((score,faces,ids.tolist()))
    used=set();faces=[]
    for _,pair,ids in sorted(candidates,key=lambda v:v[0]):
        if not any(f in used for f in pair):faces.append(ids);used.update(pair)
    faces.extend(t.tolist() for f,t in enumerate(tri.simplices) if f not in used)
    height=source.sample(xy[:,0]+center[0],xy[:,1]+center[1])
    return np.column_stack([xy,height]),faces,[[a,b,k] for (a,b),k in edges.items()],sorted(corners)


def subdivide(xy,faces,creases,corners,matrix,sharpness=1.):
    sharpness=float(np.clip(sharpness,0.,1.))
    count=len(xy);nf=len(faces);edge_faces={};vertex_faces=[[] for _ in xy];neighbors=[set() for _ in xy]
    for f,face in enumerate(faces):
        for a,b in zip(face,face[1:]+face[:1]):
            edge_faces.setdefault(tuple(sorted((a,b))),[]).append(f);neighbors[a].add(b);neighbors[b].add(a)
        for v in face:vertex_faces[v].append(f)
    edge_ids={edge:count+nf+i for i,edge in enumerate(edge_faces)};marks={tuple(sorted((a,b))):k for a,b,k in creases}
    for edge,fs in edge_faces.items():
        if len(fs)==1:marks.setdefault(edge,0)
    crease_neighbors=[[] for _ in xy]
    for a,b in marks:crease_neighbors[a].append(b);crease_neighbors[b].append(a)
    rows=[];cols=[];vals=[];xy_rows=[];xy_cols=[];xy_vals=[]
    def row(index,items,xyz=True):
        for v,w in items:
            if xyz:rows.append(index);cols.append(v);vals.append(w)
            else:xy_rows.append(index);xy_cols.append(v);xy_vals.append(w)
    def both(index,items,xy_items=None):
        if xy_items is not None:
            combined={}
            for v,w in items:combined[v]=combined.get(v,0.)+w
            for v,_ in xy_items:combined.setdefault(v,0.)
            ids=np.array(list(combined));weights=np.array(list(combined.values()));target=sum(w*xy[v] for v,w in xy_items)
            scale=max(1.,np.max(np.linalg.norm(xy[ids]-target,axis=1)));design=np.column_stack([np.ones(len(ids)),(xy[ids]-target)/scale])
            weights+=(np.array([1.,0.,0.])-weights@design)@np.linalg.pinv(design,rcond=1e-8)
            items=list(zip(ids,weights)) if abs(weights).max()<2 else xy_items
        row(index,items);row(index,items if xy_items is None else xy_items,False)
    locked=set(corners)|{v for a,b,k in creases if k==0 for v in (a,b)}
    for v in range(count):
        if v in locked or (crease_neighbors[v] and sharpness==1.):both(v,[(v,1.)]);continue
        n=len(vertex_faces[v]);items=[(v,(n-3)/n)]
        for f in vertex_faces[v]:items.extend((u,1/(n*n*len(faces[f]))) for u in faces[f])
        for u in neighbors[v]:items.extend([(v,1/(n*n)),(u,1/(n*n))])
        if crease_neighbors[v]:items=[(u,w*(1-sharpness)) for u,w in items]+[(v,sharpness)]
        both(v,items,[(v,1.)])
    for f,face in enumerate(faces):both(count+f,[(v,1/len(face)) for v in face])
    for (a,b),fs in edge_faces.items():
        index=edge_ids[(a,b)]
        if (a,b) in marks:
            if marks[(a,b)]==0:both(index,[(a,.5),(b,.5)])
            else:
                na=[v for v in crease_neighbors[a] if v!=b];nb=[v for v in crease_neighbors[b] if v!=a]
                items=[(a,9/16),(b,9/16)]
                items+= [(na[0],-1/16)] if len(na)==1 and a not in corners else [(a,-2/16),(b,1/16)]
                items+= [(nb[0],-1/16)] if len(nb)==1 and b not in corners else [(b,-2/16),(a,1/16)]
                # XYの折れを高さの曲がりへ混入させない。平面を再現する重みに補正する。
                combined={}
                for v,w in items:combined[v]=combined.get(v,0.)+w
                ids=np.array(list(combined));weights=np.array(list(combined.values()));middle=(xy[a]+xy[b])/2
                scale=max(1.,np.linalg.norm(xy[a]-xy[b]));design=np.column_stack([np.ones(len(ids)),(xy[ids]-middle)/scale])
                weights+=(np.array([1.,0.,0.])-weights@design)@np.linalg.pinv(design,rcond=1e-8)
                items=list(zip(ids,weights)) if abs(weights).max()<2 else [(a,.5),(b,.5)]
                if sharpness<1.:
                    smooth_items=[(a,.25),(b,.25)]
                    for f in fs:smooth_items.extend((v,1/(4*len(faces[f]))) for v in faces[f])
                    items=[(v,w*sharpness) for v,w in items]+[(v,w*(1-sharpness)) for v,w in smooth_items]
                both(index,items,[(a,.5),(b,.5)])
        else:
            items=[(a,.25),(b,.25)]
            for f in fs:items.extend((v,1/(4*len(faces[f]))) for v in faces[f])
            both(index,items,[(a,.5),(b,.5)])
    total=count+nf+len(edge_faces)
    transform=csr_matrix((vals,(rows,cols)),shape=(total,count));xy_transform=csr_matrix((xy_vals,(xy_rows,xy_cols)),shape=(total,count))
    new_faces=[]
    for f,face in enumerate(faces):
        for j,a in enumerate(face):new_faces.append([a,edge_ids[tuple(sorted((a,face[(j+1)%len(face)])))],count+f,edge_ids[tuple(sorted((face[j-1],a)))]])
    new_marks=[]
    for (a,b),kind in marks.items():
        e=edge_ids[(a,b)];new_marks.extend([[a,e,kind],[e,b,kind]])
    return xy_transform@xy,new_faces,new_marks,corners,transform@matrix


def generate(args):
    args.output.mkdir(parents=True,exist_ok=True);target=args.output/'ridge.sqlite3'
    if target.exists() and not getattr(args,'resume',False):raise RuntimeError('新しい出力先を指定してください')
    source=Source(args.source);meta=sqlite3.connect(args.base.resolve().as_uri()+'?mode=ro',uri=True);db=sqlite3.connect(target);db.execute('CREATE TABLE IF NOT EXISTS surfaces(region TEXT,quality TEXT,info TEXT,model BLOB,PRIMARY KEY(region,quality)) WITHOUT ROWID')
    reports=[json.loads(r[0]) for r in db.execute('SELECT info FROM surfaces')]
    for name,raw in meta.execute('SELECT name,info FROM regions ORDER BY name'):
        center=np.array(json.loads(raw)['center']);axis=np.linspace(-16000,16000,257);gx,gy=np.meshgrid(axis,axis);h=source.sample(gx+center[0],gy+center[1]);smooth=gaussian_filter(h,.7)
        peaks=np.argwhere((h==maximum_filter(h,9))&((h-gaussian_filter(h,5))>max(30.,.06*np.ptp(h))))
        # 地域中心近傍の山頂も評価する。標高は元DEMから取得する。
        local=h[120:137,120:137];iy,ix=np.unravel_index(np.argmax(local),local.shape);peaks=np.vstack([peaks,[iy+120,ix+120]])
        peak_xy=np.column_stack([axis[peaks[:,1]],axis[peaks[:,0]]])
        selected=[v for v in SETTINGS if (v['id']=='half')==getattr(args,'half_budget',False)]
        for settings in selected:
            if db.execute('SELECT 1 FROM surfaces WHERE region=? AND quality=?',(name,settings['id'])).fetchone():continue
            start=time.monotonic();print(name,settings['id'],'骨格抽出',flush=True)
            lines=[]
            for kind in [1,2]:
                for line in flow_lines(h,settings['flow'],kind):
                    q=simplify(line,settings['xy'],settings['height']*.5)
                    if len(q)>1:lines.append((q,kind))
            budget_info={}
            if settings['id']=='half':
                _,base_xy,_,base_h,base_faces,_=load_level(meta,name,14)
                base_model=dict(vertices=np.column_stack((base_xy*UNIT-center,base_h)).astype('<f4').tolist(),faces=base_faces.tolist(),span=SPAN)
                base_bytes=len(zlib.compress(json.dumps(base_model,separators=(',',':')).encode(),6));target_bytes=base_bytes*.5
                best=None;scale=getattr(args,'initial_scale',1.)
                for attempt in range(7):
                    trial=dict(settings,flow=max(2,int(16/scale)),xy=max(8.,60./scale),height=max(.5,8./scale),extra=min(16384,int(2048*scale)),querySize=257,batch=128,spacing=max(65.,220./scale))
                    trial_lines=[]
                    for kind in [1,2]:
                        for line in flow_lines(h,trial['flow'],kind):
                            q=simplify(line,trial['xy'],trial['height']*.5)
                            if len(q)>1:trial_lines.append((q,kind))
                    candidate=constrained_mesh(trial_lines,peak_xy,source,center,trial)
                    vv,ff0,cc0,kk=candidate
                    sample=dict(vertices=vv.astype('<f4').tolist(),faces=ff0,creases=cc0,corners=kk,span=SPAN)
                    size=len(zlib.compress(json.dumps(sample,separators=(',',':')).encode(),6))
                    print(name,'容量調整',attempt,'制御点',len(vv),'bytes',size,'target',target_bytes,flush=True)
                    if best is None or abs(size-target_bytes)<abs(best[0]-target_bytes):best=(size,candidate,trial)
                    if abs(size/target_bytes-1)<.1:break
                    scale*=float(np.clip((target_bytes/size)**.75,.5,3.))
                _,(vertices,faces,creases,corners),settings=best
                budget_info=dict(referenceMeshBytes=base_bytes,targetBytes=target_bytes,referenceFormat='float32 local XYZ + triangle indices, JSON + zlib6',referenceZ=14)
            else:
                vertices,faces,creases,corners=constrained_mesh(lines,peak_xy,source,center,settings)
            matrix=eye(len(vertices),format='csr');xy=vertices[:,:2].copy();ff=faces;cc=creases
            fit_steps=1 if len(vertices)>12000 else 2
            for _ in range(fit_steps):xy,ff,cc,_,matrix=subdivide(xy,ff,cc,corners,matrix)
            target_h=source.sample(xy[:,0]+center[0],xy[:,1]+center[1]);fixed=np.unique(np.r_[np.array(creases)[:,:2].ravel(),corners]).astype(int);free=np.setdiff1d(np.arange(len(vertices)),fixed)
            if len(free):
                residual=target_h-matrix[:,fixed]@vertices[fixed,2]
                result=lsqr(matrix[:,free],residual,damp=.01,atol=1e-7,btol=1e-7,iter_lim=500)[0]
                vertices[free,2]=np.clip(result,vertices[free,2]-2*settings['height'],vertices[free,2]+2*settings['height'])
            xy,ff,cc,_,matrix=subdivide(xy,ff,cc,corners,matrix);height=matrix@vertices[:,2]
            minimum_area=float('inf')
            for face in ff:
                p=xy[face]
                for j in range(1,len(face)-1):
                    a,b=p[j]-p[0],p[j+1]-p[0];minimum_area=min(minimum_area,float(a[0]*b[1]-a[1]*b[0]))
            if minimum_area<=0:raise RuntimeError('細分割面のXYが反転しました')
            # 一様評価点で、3回細分割した表示形状との誤差を参考照合する。
            tri=Delaunay(xy);qaxis=np.linspace(-15999,15999,257);qx,qy=np.meshgrid(qaxis,qaxis);q=np.column_stack([qx.ravel(),qy.ravel()]);si=tri.find_simplex(q);t=tri.transform[si];bary=np.einsum('nij,nj->ni',t[:,:2],q-t[:,2]);bary=np.column_stack([bary,1-bary.sum(1)]);values=np.sum(height[tri.simplices[si]]*bary,axis=1);reference=source.sample(q[:,0]+center[0],q[:,1]+center[1]);error=values-reference
            model=dict(vertices=vertices.astype('<f4').tolist(),faces=faces,creases=creases,corners=corners,span=SPAN)
            payload=json.dumps(model,separators=(',',':')).encode();packed=zlib.compress(payload,6)
            info=dict(region=name,quality=settings['id'],label=settings['label'],controls=len(vertices),patches=len(faces),quads=sum(len(f)==4 for f in faces),ridges=sum(k==1 for _,_,k in creases),valleys=sum(k==2 for _,_,k in creases),protectedPeaks=len(corners)-4,storedBytes=len(packed),jsonBytes=len(payload),rmse=float(np.sqrt(np.mean(error**2))),maxError=float(abs(error).max()),evaluationGridMeters=125.,evaluationSubdivision=fit_steps+1,fitSubdivision=fit_steps,**budget_info,seconds=time.monotonic()-start,settings=settings,minimumProjectedDoubleArea=minimum_area,scheme='fixed-xy-affine-height-v1')
            db.execute('INSERT INTO surfaces VALUES(?,?,?,?)',(name,settings['id'],json.dumps(info),packed));db.commit();reports.append(info);print(json.dumps(info),flush=True);(args.output/'progress.json').write_text(json.dumps(info),encoding='utf-8')
    db.close();(args.output/'report.json').write_text(json.dumps(dict(levels=reports,sqliteBytes=target.stat().st_size),indent=2),encoding='utf-8');(args.output/'progress.json').write_text('{"complete":true}',encoding='utf-8')


def preview(model,divisions,sharpness=1.):
    """表示用の細分割・法線のステンシル。元DEMは参照しない。"""
    vertices=np.array(model['vertices'],float);xy=vertices[:,:2];faces=model['faces'];creases=model['creases'];corners=model['corners'];matrix=eye(len(vertices),format='csr')
    requested=int(divisions);level=int(np.log2(requested));actual=level
    face_limit=1000000 if model.get('info',{}).get('quality')=='half' else 2000000
    while actual>0 and len(faces)*4**actual*2>face_limit:actual-=1
    for _ in range(actual):xy,faces,creases,corners,matrix=subdivide(xy,faces,creases,corners,matrix,sharpness)
    marks={tuple(sorted((a,b))) for a,b,k in creases if k==0 or sharpness>=1.};edge_faces={};vf=[[] for _ in xy]
    for f,face in enumerate(faces):
        for a,b in zip(face,face[1:]+face[:1]):edge_faces.setdefault(tuple(sorted((a,b))),[]).append(f)
        for v in face:vf[v].append(f)
    connections=[[] for _ in xy]
    for (a,b),fs in edge_faces.items():
        if len(fs)==2 and (a,b) not in marks:connections[a].append(fs);connections[b].append(fs)
    group_vertex=[];corner_groups={};gradient_rows=[];gradient_cols=[];gradient_x=[];gradient_y=[];first_group={}
    for v,incident in enumerate(vf):
        parent={f:f for f in incident}
        def find(f):
            while parent[f]!=f:parent[f]=parent[parent[f]];f=parent[f]
            return f
        if v not in corners or sharpness<1.:
            for a,b in connections[v]:parent[find(a)]=find(b)
        groups={}
        for f in incident:groups.setdefault(find(f),[]).append(f)
        for fs in groups.values():
            row=len(group_vertex);group_vertex.append(v);first_group.setdefault(v,row)
            for f in fs:corner_groups[(v,f)]=row
            neighbors=sorted({u for f in fs for u in faces[f]}-{v});delta=xy[neighbors]-xy[v]
            normal=delta.T@delta
            inverse=np.linalg.pinv(normal,rcond=1e-10)@delta.T
            gradient_rows.extend([row]*(len(neighbors)+1));gradient_cols.extend(neighbors+[v]);gradient_x.extend([*inverse[0],-inverse[0].sum()]);gradient_y.extend([*inverse[1],-inverse[1].sum()])
    n=len(group_vertex)
    dx=csr_matrix((gradient_x,(gradient_rows,gradient_cols)),shape=(n,len(xy)))@matrix
    dy=csr_matrix((gradient_y,(gradient_rows,gradient_cols)),shape=(n,len(xy)))@matrix
    position=matrix[group_vertex]
    arrays=[position.tocoo(),dx.tocoo(),dy.tocoo()];keys=np.concatenate([a.row.astype(np.int64)*len(vertices)+a.col for a in arrays]);unique,inverse=np.unique(keys,return_inverse=True)
    weights=np.zeros((len(unique),4),'<f4');weights[:,0]=unique%len(vertices);at=0
    for k,a in enumerate(arrays):np.add.at(weights[:,k+1],inverse[at:at+len(a.data)],a.data);at+=len(a.data)
    counts=np.bincount(unique//len(vertices),minlength=n).astype('<u4')
    if counts.max()>256:raise RuntimeError(f'ステンシルが大きすぎます: {counts.max()}')
    lookup=np.column_stack([np.r_[0,counts.cumsum()[:-1]],counts]).astype('<u4');triangles=[]
    for f,face in enumerate(faces):
        ids=[corner_groups[(v,f)] for v in face]
        for j in range(1,len(ids)-1):triangles.extend([ids[0],ids[j],ids[j+1]])
    lines={1:[],2:[]}
    for a,b,k in creases:
        if k in lines:lines[k].extend([first_group[a],first_group[b]])
    def encoded(a,dtype):return base64.b64encode(np.asarray(a,dtype=dtype).tobytes()).decode()
    result=dict(xy=encoded(xy[group_vertex],'<f4'),lookup=encoded(lookup,'<u4'),weights=encoded(weights,'<f4'),indices=encoded(triangles,'<u4'),ridgeLines=encoded(lines[1],'<u4'),valleyLines=encoded(lines[2],'<u4'),peaks=encoded([first_group[v] for v in corners],'<u4'),controls=encoded(vertices[:,2],'<f4'),vertexCount=n,entries=len(weights),triangles=len(triangles)//3,division=2**actual,requestedDivision=requested,maxStencil=int(counts.max()),sharpness=sharpness)
    result['derivedBytes']=n*16+weights.nbytes+len(triangles)*4+sum(len(v) for v in lines.values())*4+len(corners)*4+vertices.shape[0]*4
    xx=np.linspace(-15680,15680,513);query=np.column_stack([xx,np.zeros(len(xx))]);tri=Delaunay(xy);si=tri.find_simplex(query);t=tri.transform[si];bary=np.einsum('nij,nj->ni',t[:,:2],query-t[:,2]);bary=np.column_stack([bary,1-bary.sum(1)]);height=matrix@vertices[:,2]
    result['profile']=np.sum(height[tri.simplices[si]]*bary,axis=1).tolist()
    return result


def reference_mesh_sizes(base,region):
    """表示用の元標高メッシュを各レベル独立・同じ保存条件で測る。"""
    with sqlite3.connect(Path(base).resolve().as_uri()+'?mode=ro',uri=True) as db:
        center=np.array(json.loads(db.execute('SELECT info FROM regions WHERE name=?',(region,)).fetchone()[0])['center'])
        result={}
        for z in range(6,15):
            _,xy,_,h,faces,_=load_level(db,region,z)
            model=dict(vertices=np.column_stack((xy*UNIT-center,h)).astype('<f4').tolist(),faces=faces.tolist(),span=SPAN)
            result[z]=len(zlib.compress(json.dumps(model,separators=(',',':')).encode(),6))
    return dict(region=region,bytesByZ=result,format='local float32 XYZ + triangles, JSON + zlib6')


def serve(args):
    reference_sizes=functools.lru_cache(maxsize=3)(lambda region:json.dumps(reference_mesh_sizes(args.base,region)).encode())
    @functools.lru_cache(maxsize=12)
    def packet(region,quality,division,sharpness=1.):
        db_path=args.output/'ridge.sqlite3'
        if quality!='half' and getattr(args,'half_budget',False):db_path=Path('build/ridge-surface-prototype-v4/ridge.sqlite3')
        with sqlite3.connect(db_path.resolve().as_uri()+'?mode=ro',uri=True) as db:row=db.execute('SELECT info,model FROM surfaces WHERE region=? AND quality=?',(region,quality)).fetchone()
        if row is None:raise ValueError('未生成の曲面です')
        model=json.loads(zlib.decompress(row[1]));model['info']=json.loads(row[0]);model['mesh']=preview(model,division,sharpness)
        model['summit']=min((model['vertices'][v] for v in model['corners']),key=lambda p:p[0]*p[0]+p[1]*p[1])
        return gzip.compress(json.dumps(model).encode(),1)
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            try:
                url=urlparse(self.path);q=parse_qs(url.query);encoding=None
                if url.path=='/api/ridge':body=packet(q.get('region',['yari'])[0],q.get('quality',['medium'])[0],int(q.get('division',['4'])[0]),float(q.get('sharpness',['1'])[0]));mime='application/json';encoding='gzip'
                elif url.path=='/api/mesh-sizes':body=reference_sizes(q.get('region',['yari'])[0]);mime='application/json'
                elif url.path.startswith('/api/'):
                    with urllib.request.urlopen('http://127.0.0.1:61468'+self.path,timeout=120) as r:body=r.read();mime=r.headers['Content-Type'];encoding=r.headers.get('Content-Encoding')
                elif url.path=='/':
                    with urllib.request.urlopen('http://127.0.0.1:61468/') as r:html=r.read().decode()
                    html=html.replace('山の形を少数の自由配置曲面で近似','山頂・尾根・谷を保持する細分割曲面').replace('<option value="features" selected>','<option value="ridge" selected>骨格保持の四角形曲面</option><option value="features">')
                    html=html.replace('<button id="reset">','<label>骨格品質 <select id="ridgeQuality">'+''.join(f'<option value="{s["id"]}"'+(' selected' if s['id']==('half' if getattr(args,'half_budget',False) else 'medium') else '')+f'>{s["label"]}</option>' for s in SETTINGS if s['id']!='half' or getattr(args,'half_budget',False))+'</select></label><label><input type="checkbox" id="ridgeLines">尾根・谷線</label><button id="ridgeFocus">山頂へ接近</button><button id="reset">')
                    html=html.replace('<label><input type="checkbox" id="ridgeLines">','<label>尾根・谷の折れ <select id="ridgeSharpness"><option value="0">なめらか</option><option value="0.25" selected>弱く残す</option><option value="1">鋭く残す（従来）</option></select></label><label><input type="checkbox" id="ridgeLines">')
                    html=html.replace('<script src="/lpf_curve_compare.js">','<script>window.ridgeMeshSizesEnabled=true;</script><script src="/ridge_surface.js"></script><script src="/lpf_curve_compare.js">');a=html.index('<div id="note">');b=html.index('</div>',a)
                    html=html[:a]+'<div id="note">山頂と外周を固定し、尾根・谷の折れを調整します。「弱く残す」は高さの補間を山腹側へなじませ、骨格をまたいで法線をつなぎます。「鋭く残す」は従来の形状です。山腹は四角形主体の細分割曲面。骨格線はD8の流路と逆地形の流路から推定した候補です。線を横断して面を結合しません。表示は生成済みポリゴン。左は元TIN。比較用RBF・Bスプラインも保持。LOD・地域境界接合は未実装。</div>'+html[b+6:];body=html.encode();mime='text/html; charset=utf-8'
                elif url.path in ['/ridge_surface.js','/feature_surface.js','/bspline_surface.js','/lpf_curve_compare.js','/lpf_curve_raycast.js']:body=Path(__file__).with_name(url.path[1:]).read_bytes();mime='text/javascript; charset=utf-8'
                else:self.send_error(404);return
                self.send_response(200);self.send_header('Content-Type',mime);self.send_header('Content-Length',str(len(body)));self.send_header('Cache-Control','no-cache')
                if encoding:self.send_header('Content-Encoding',encoding)
                self.end_headers();self.wfile.write(body)
            except Exception as e:self.send_error(400,str(e))
        def log_message(self,*args):pass
    print(f'http://127.0.0.1:{args.port}/',flush=True);ThreadingHTTPServer(('127.0.0.1',args.port),Handler).serve_forever()


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('action',choices=['generate','serve']);p.add_argument('--output',type=Path,default=Path('build/ridge-surface-prototype-v4'));p.add_argument('--source',type=Path,default=Path('build/terrain-lod.sqlite3'));p.add_argument('--base',type=Path,default=Path('build/lpf-curve-smooth-prototype/comparison.sqlite3'));p.add_argument('--port',type=int,default=61469);p.add_argument('--half-budget',action='store_true');p.add_argument('--resume',action='store_true');p.add_argument('--initial-scale',type=float,default=1.)
    a=p.parse_args();dict(generate=generate,serve=serve)[a.action](a)
