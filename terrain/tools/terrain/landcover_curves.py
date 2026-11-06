"""Shared quadratic-Bezier corner fillets, bounded by distance to foreign edges."""
from functools import lru_cache
from pathlib import Path
from io import BytesIO
import json,sqlite3,zlib,gzip,time,threading
from contextlib import closing
import numpy as np
import shapely
from shapely.geometry import Polygon,MultiPolygon,mapping

LOCK=threading.Lock()
def polygons(g):
    if g.geom_type=="Polygon":return [g]if not g.is_empty else[]
    if g.geom_type=="MultiPolygon":return list(g.geoms)
    return []
def graph(groups,side=128000.):
    rings=[];meta=[];offsets=[0];arrays=[]
    for code,g in enumerate(groups):
        for poly_id,poly in enumerate(polygons(g)):
            for ring_id,ring in enumerate([poly.exterior,*poly.interiors]):
                a=np.asarray(ring.coords,dtype=np.float64)[:-1,:2]
                arrays.append(a);meta.append((code,poly_id,ring_id));offsets.append(offsets[-1]+len(a))
    all_points=np.concatenate(arrays)
    points,inverse=np.unique(all_points,axis=0,return_inverse=True)
    edges=[]
    for a,b in zip(offsets[:-1],offsets[1:]):
        ids=inverse[a:b];rings.append(ids)
        edges.append(np.column_stack([ids,np.roll(ids,-1)]))
    segments=np.unique(np.sort(np.concatenate(edges),axis=1),axis=0)
    segments=segments[segments[:,0]!=segments[:,1]]
    count=len(points);degree=np.bincount(segments.ravel(),minlength=count)
    first=np.full(count,count,np.int64);last=np.zeros(count,np.int64)
    for a,b in [(segments[:,0],segments[:,1]),(segments[:,1],segments[:,0])]:
        np.minimum.at(first,a,b);np.maximum.at(last,a,b)
    first=np.minimum(first,count-1)
    prev=points[first]-points;next_=points[last]-points
    l0=np.linalg.norm(prev,axis=1);l1=np.linalg.norm(next_,axis=1)
    u=prev/np.maximum(l0[:,None],1e-30);v=next_/np.maximum(l1[:,None],1e-30)
    cut=.45*np.minimum(l0,l1)
    boundary=np.any((np.abs(points)<1e-7)|(np.abs(points-side)<1e-7),axis=1)
    straight=np.sum(u*v,axis=1)<-0.999999
    cut[(degree!=2)|boundary|straight]=0.
    tree=shapely.STRtree(shapely.linestrings(points[segments]))
    active=np.flatnonzero(cut>1e-8)
    # At most 35% of the nearest non-incident-edge clearance. Local deformation
    # disks cannot touch other edges or another corner's deformation disk.
    for start in range(0,len(active),2048):
        ids=active[start:start+2048];p=points[ids];r=cut[ids]/.35
        pair=tree.query(shapely.box(p[:,0]-r,p[:,1]-r,p[:,0]+r,p[:,1]+r))
        local,edge=pair
        keep=(segments[edge,0]!=ids[local])&(segments[edge,1]!=ids[local])
        local=local[keep];edge=edge[keep]
        a=points[segments[edge,0]];d=points[segments[edge,1]]-a
        q=p[local]-a
        t=np.clip(np.sum(q*d,axis=1)/np.maximum(np.sum(d*d,axis=1),1e-30),0.,1.)
        distance=np.linalg.norm(q-t[:,None]*d,axis=1)
        minimum=np.full(len(ids),np.inf);np.minimum.at(minimum,local,distance)
        cut[ids]=np.minimum(cut[ids],.35*minimum)
    cut[cut<1e-6]=0.
    return {"points":points,"cut":cut,"first":first,"last":last,"rings":rings,"meta":np.asarray(meta,np.uint32),"offsets":np.asarray(offsets,np.uint32),"ids":inverse.astype(np.uint32),"side":side}

def evaluate(g,strength=.75,subdivisions=4):
    p=g["points"];r=g["cut"]*strength
    a=p+(p[g["first"]]-p)*((r/np.maximum(np.linalg.norm(p[g["first"]]-p,axis=1),1e-30))[:,None])
    b=p+(p[g["last"]]-p)*((r/np.maximum(np.linalg.norm(p[g["last"]]-p,axis=1),1e-30))[:,None])
    t=np.linspace(0.,1.,subdivisions+1)[None,:,None]
    corners=(1-t)**2*a[:,None,:]+2*(1-t)*t*p[:,None,:]+t*t*b[:,None,:]
    collected=[{}for _ in range(5)]
    for ids,meta in zip(g["rings"],g["meta"]):
        code,poly,ring=map(int,meta)
        previous=np.roll(ids,1)
        forward=previous==g["first"][ids]
        q=corners[ids].copy();q[~forward]=q[~forward,::-1,:]
        mask=np.ones(q.shape[:2],bool);mask[r[ids]==0,1:]=False
        pts=q[mask]
        collected[code].setdefault(poly,{})[ring]=pts
    groups=[]
    for code,polys in enumerate(collected):
        items=[]
        for index,rings in sorted(polys.items()):
            poly=Polygon(rings[0],[r for i,r in sorted(rings.items())if i])
            if not poly.is_valid:raise ValueError(f"Curved polygon invalid: {code}/{index}")
            items.append(poly)
        group=MultiPolygon(items)
        if not group.is_valid:raise ValueError(f"Curved class invalid: {code}")
        groups.append(group)
    area=sum(poly.area for poly in groups);expected=g["side"]**2
    if abs(area-expected)>expected*1e-9:raise ValueError("Curved coverage area changed")
    return groups

@lru_cache(maxsize=3)
def load_graph(data,level):
    start=time.monotonic();root=Path(data)
    with closing(sqlite3.connect((root/'landcover-vectors.sqlite3').as_uri()+'?mode=ro',uri=True))as db:
        groups=[shapely.from_wkb(zlib.decompress(row[0]))for code in range(5)
                for row in db.execute("SELECT geometry FROM features WHERE level=? AND class=?",(level,code))]
    bounds=json.loads((root/'manifest.json').read_bytes())['bounds']
    result=graph(groups,bounds[2]-bounds[0])
    controls=dict(points=result['points'],cut=result['cut'],ring_ids=result['ids'],
                  ring_offsets=result['offsets'],ring_meta=result['meta'])
    if result['side'] <= 128000.01:
        folder=root/'curves';folder.mkdir(exist_ok=True)
        path=folder/f'level-{level}-quadratic.npz'
        if not path.exists():
            np.savez_compressed(path,**controls)
        result['controlBytes']=path.stat().st_size
    else:
        with BytesIO() as temporary:
            np.savez_compressed(temporary,**controls)
            result['controlBytes']=temporary.tell()
    result['buildSeconds']=time.monotonic()-start
    return result

@lru_cache(maxsize=5)
def curved(data,level,strength,subdivisions):
    with LOCK:
        g=load_graph(data,level)
        start=time.monotonic()
        groups=evaluate(g,strength,subdivisions)
        features=[{"type":"Feature","properties":{"class":code},"geometry":mapping(poly)}for code,poly in enumerate(groups)]
        info={"method":"shared-quadratic-bezier","strength":strength,"subdivisions":subdivisions,
              "controlBytes":g["controlBytes"],"controlPoints":len(g["points"]),
              "roundedCorners":int(np.count_nonzero(g["cut"])),"maximumCutM":float(g["cut"].max()),
              "graphSeconds":g["buildSeconds"],"evaluateSeconds":time.monotonic()-start,
              "coverageAreaErrorM2":abs(sum(p.area for p in groups)-g["side"]**2)}
        raw=json.dumps({"type":"FeatureCollection","features":features,"curveInfo":info},separators=(',',':')).encode()
        return gzip.compress(raw,9,mtime=0),groups,info
