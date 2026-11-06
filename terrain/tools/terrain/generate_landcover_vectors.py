"""Fuji 128 km land-cover coverage: shared boundaries, five vector LODs, separate ocean."""
import argparse
from contextlib import closing
import gzip
import json
import math
from pathlib import Path
import sqlite3
import time
import zlib
import numpy as np
from PIL import Image
import rasterio
from rasterio.features import shapes
from rasterio.transform import from_bounds
from rasterio.warp import reproject, Resampling
import shapely
from shapely.geometry import box, shape, mapping
from shapely.affinity import affine_transform

ROOT=Path(__file__).resolve().parents[2]
R=6378137.
PALETTE={0:("#b8be9e","その他"),1:("#527046","森林"),2:("#aba089","砂礫地・裸地"),3:("#4c84a6","河川・湖沼"),4:("#86b4cb","海")}
LEVELS=[(0,0.,0.),(1,25.,2500.),(2,75.,10000.),(3,200.,40000.),(4,500.,160000.)]
ATTRIBUTION="© ESA WorldCover project 2021 / Contains modified Copernicus Sentinel data (2021) processed by ESA WorldCover consortium"
def project(lon,lat):
    return R*math.radians(lon),R*math.asinh(math.tan(math.radians(lat)))
def classify(classes):
    out=np.zeros(classes.shape,np.uint8)
    out[np.isin(classes,[10,95])]=1
    out[classes==60]=2
    out[classes==80]=3
    return out
def all_polygons(g):
    if g.geom_type=="Polygon":
        if not g.is_empty:yield g
    elif hasattr(g,"geoms"):
        for p in g.geoms:yield from all_polygons(p)
def coverage(labels,transform,land,bounds):
    """Node all boundaries once, including coast intersections and collinear T junctions."""
    domain=box(*bounds)
    candidates=[]
    shapely.prepare(land)
    for n,(geometry,value) in enumerate(shapes(labels,transform=transform,connectivity=4)):
        g=shape(geometry)
        if not shapely.covers(land,g):
            if shapely.disjoint(land,g):continue
            g=g.intersection(land)
        if not g.is_empty:candidates.extend(all_polygons(g))
        if n%10000==0:print("Coast clipping",n,flush=True)
    print("Building shared boundary graph",len(candidates),flush=True)
    candidates.extend(all_polygons(domain.difference(land)))
    # Polygonizing one noded planar graph constructs non-overlapping faces
    # with identical shared vertices directly, without quadratic pair checks.
    network=shapely.union_all(shapely.boundary(np.asarray(candidates,dtype=object)))
    faces=np.asarray(list(shapely.polygonize(shapely.get_parts(network)).geoms),dtype=object)
    centers=shapely.point_on_surface(faces)
    xy=shapely.get_coordinates(centers)
    cols=np.floor((xy[:,0]-bounds[0])/(bounds[2]-bounds[0])*labels.shape[1]).astype(int).clip(0,labels.shape[1]-1)
    rows=np.floor((bounds[3]-xy[:,1])/(bounds[3]-bounds[1])*labels.shape[0]).astype(int).clip(0,labels.shape[0]-1)
    codes=labels[rows,cols].copy()
    codes[~shapely.contains(land,centers)]=4
    if not shapely.is_valid(faces).all():raise ValueError("Invalid input face")
    if abs(float(shapely.area(faces).sum())-domain.area)>domain.area*1e-10:raise ValueError("Initial coverage has gaps")
    return faces,codes
def adjacency(faces):
    neighbors=[{}for _ in faces];edges={}
    for i,g in enumerate(faces):
        for ring in [g.exterior,*g.interiors]:
            points=list(ring.coords)
            for a,b in zip(points,points[1:]):
                if a==b:continue
                key=(a,b)if a<b else(b,a)
                previous=edges.pop(key,None)
                if previous is None:edges[key]=i
                elif previous!=i:
                    length=math.hypot(b[0]-a[0],b[1]-a[1])
                    neighbors[i][previous]=neighbors[i].get(previous,0.)+length
                    neighbors[previous][i]=neighbors[previous].get(i,0.)+length
    return neighbors
def simplify_level(faces,codes,tolerance,minimum,domain,neighbors=None):
    ids=codes.copy()
    fb=shapely.bounds(faces);b=domain.bounds
    touching=np.isclose(fb[:,0],b[0],atol=1e-6,rtol=0)|np.isclose(fb[:,1],b[1],atol=1e-6,rtol=0)|np.isclose(fb[:,2],b[2],atol=1e-6,rtol=0)|np.isclose(fb[:,3],b[3],atol=1e-6,rtol=0)
    if minimum:
        areas=shapely.area(faces);parent=np.arange(len(faces))
        if neighbors is None:neighbors=adjacency(faces)
        for i in np.argsort(areas,kind="stable"):
            if ids[i]>=3 or areas[i]>=minimum or touching[i]:continue
            choices=[(length,areas[j],-j,j)for j,length in neighbors[i].items()
                     if ids[j]<3 and (areas[j]>areas[i]+1e-6 or(abs(areas[j]-areas[i])<=1e-6 and j<i))]
            if choices:parent[i]=max(choices)[3]
        def root(i):
            while parent[i]!=i:
                parent[i]=parent[parent[i]];i=parent[i]
            return i
        ids=np.array([codes[root(i)]for i in range(len(faces))],np.uint8)
        merged=[shapely.coverage_union_all(faces[ids==c])for c in range(5)]
        network=shapely.union_all(shapely.boundary(np.asarray(merged,dtype=object)))
        faces=shapely.get_parts(shapely.polygonize(shapely.get_parts(network)))
        points=shapely.point_on_surface(faces);ids=np.zeros(len(faces),np.uint8)
        for c,g in enumerate(merged):
            shapely.prepare(g);ids[shapely.contains(g,points)]=c
    # Water is not removed by the forest/bare-island threshold; its boundaries get a tighter tolerance.
    simplified=faces.copy()
    if tolerance:
        # First simplify the full coverage slightly. Then simplify only dry-land
        # classes, keeping all water/sea boundaries as the fixed boundary of that subset.
        simplified=shapely.coverage_simplify(faces,min(tolerance,16.),simplify_boundary=False)
        active=ids<3
        simplified[active]=shapely.coverage_simplify(simplified[active],tolerance,simplify_boundary=False)
    if not shapely.is_valid(simplified).all():raise ValueError("Invalid simplified face")
    # Matching non-overlapping coverage permits a linear coverage union.
    # A general overlay union against the background's many holes is unnecessary.
    grouped=[shapely.coverage_union_all(simplified[ids==c])for c in range(5)]
    gap=abs(float(shapely.area(simplified).sum())-domain.area)
    overlap=0.
    if gap>domain.area*1e-10:raise ValueError("LOD coverage area changed")
    return grouped,{"gapAreaM2":gap,"overlapAreaM2":max(0.,overlap)}
def relief(bounds,out,size=768):
    """Reference relief only: source DEM samples, not distributed terrain geometry."""
    from ridges import decode_png
    job=ROOT/'build/national-shared-z6-z13-v1/job.json'
    if not job.exists():return None
    cache=Path(json.loads(job.read_text())["identity"]["cache"])
    world=2*math.pi*R
    x=np.linspace(bounds[0],bounds[2],size);y=np.linspace(bounds[3],bounds[1],size)
    xx,yy=np.meshgrid(x,y)
    px=(xx+world/2)/world*(16384*256)
    py=(world/2-yy)/world*(16384*256)
    tx=np.floor(px/256).astype(int);ty=np.floor(py/256).astype(int)
    height=np.zeros((size,size),np.float32)
    for a,b in sorted(set(zip(tx.ravel(),ty.ravel()))):
        path=cache/'14'/str(a)/(str(b)+'.png')
        if not path.exists():continue
        values=decode_png(path.read_bytes())
        mask=(tx==a)&(ty==b)
        ix=(px[mask]%256).astype(int);iy=(py[mask]%256).astype(int)
        height[mask]=np.nan_to_num(values[iy,ix],nan=0.)
    dy,dx=np.gradient(height,(bounds[2]-bounds[0])/(size-1))
    light=(-.55*dx+.65*dy+1)/np.sqrt(1+dx*dx+dy*dy)/math.sqrt(.55**2+.65**2+1)
    shade=np.uint8(np.clip((.39+.70*light)*255,70,255))
    Image.fromarray(shade).save(out/'relief.png')
    return {"file":"relief.png","source":"国土地理院元DEM・参考陰影のみ。地表分類には使用しない。","size":size}
def generate(out,size):
    start=time.monotonic();out.mkdir(parents=True,exist_ok=True)
    if (out/'manifest.json').exists():raise FileExistsError("Use a new output directory")
    center=project(138.727,35.36);side=128000.
    bounds=[center[0]-side/2,center[1]-side/2,center[0]+side/2,center[1]+side/2]
    source=ROOT/'build/terrain-worldcover-source/ESA_WorldCover_10m_2021_v200_N33E138_Map.tif'
    labels=np.zeros((size,size),np.uint8);transform=from_bounds(*bounds,size,size)
    with rasterio.open(source)as ds:
        reproject(rasterio.band(ds,1),labels,src_transform=ds.transform,src_crs=ds.crs,
                  dst_transform=transform,dst_crs="EPSG:3857",resampling=Resampling.nearest,num_threads=4,warp_mem_limit=256)
    classes=labels.copy();labels=classify(classes)
    print("WorldCover sampled",size,"grid; pixel metres",side/size,flush=True)
    land=shapely.from_wkb((ROOT/'build/terrain-coast-source.union.wkb').read_bytes()).intersection(box(*bounds))
    # Bounded spatial batches avoid quadratic validation against a single
    # enormous background polygon with tens of thousands of holes.
    batches=[];k=8;step=size//k
    if size%k:raise ValueError("Grid size must be divisible by 8")
    for iy in range(k):
        for ix in range(k):
            tile=box(bounds[0]+ix*side/k,bounds[3]-(iy+1)*side/k,bounds[0]+(ix+1)*side/k,bounds[3]-iy*side/k)
            sub=labels[iy*step:(iy+1)*step,ix*step:(ix+1)*step]
            cache=out/f"batch-{size}-{iy}-{ix}"
            if cache.with_suffix(".wkb").exists()and cache.with_suffix(".codes").exists():
                faces=shapely.get_parts(shapely.from_wkb(cache.with_suffix(".wkb").read_bytes()))
                codes=np.frombuffer(cache.with_suffix(".codes").read_bytes(),np.uint8).copy()
            else:
                faces,codes=coverage(sub,from_bounds(*tile.bounds,step,step),land.intersection(tile),tile.bounds)
                cache.with_suffix(".wkb").write_bytes(shapely.to_wkb(shapely.GeometryCollection(faces)))
                cache.with_suffix(".codes").write_bytes(codes.tobytes())
            batches.append((faces,codes,tile,adjacency(faces)))
        print("Shared coverage strips",iy+1,"/",k,flush=True)
    print("Noded coverage",sum(len(f)for f,_,_,_ in batches),"faces",flush=True)
    # Palette reference is for comparison only; distribution vectors do not depend on it.
    ocean=~shapely.contains_xy(land,*np.meshgrid(np.linspace(bounds[0],bounds[2],size),np.linspace(bounds[3],bounds[1],size)))
    reference=labels.copy();reference[ocean]=4
    palette=Image.fromarray(reference,'P');rgb=[]
    for i in range(256):
        color=PALETTE.get(i,PALETTE[0])[0];rgb.extend(bytes.fromhex(color[1:]))
    palette.putpalette(rgb);palette.save(out/'reference.png',optimize=True)
    manifest={"region":"fuji","title":"富士山128 km・地表ベクトルLOD","crs":"EPSG:3857",
              "bounds":bounds,"side":side,"centerLonLat":[138.727,35.36],
              "sourceGrid":size,"sourcePixelProjectedM":side/size,"classes":{k:{"color":v[0],"label":v[1]}for k,v in PALETTE.items()},
              "source":"ESA WorldCover 2021 v200","sourceFile":source.name,
              "sourceURL":"https://esa-worldcover.org/en/data-access","attribution":ATTRIBUTION,"license":"CC BY 4.0",
              "coastSource":"国土地理院最適化ベクトルタイル・海域を除いた陸域",
              "coastURL":"https://maps.gsi.go.jp/development/vt_expt.html",
              "classification":{"forest":[10,95],"bare":[60],"inlandWater":[80],"other":"その他すべて。市街地は区別しない。","sea":"国土地理院の海岸線でWorldCover水域と分離"},
              "referenceBytes":(out/'reference.png').stat().st_size,"levels":[],"tiled":False,
              "processingBatches":64,"fixedBatchBoundaryM":16000,"notes":["128 kmはWebメルカトル投影上の長さ。実距離は緯度により短い。","境界簡略化はVisvalingam系の目安であり最大位置誤差の保証ではない。","小さな陸地領域を共有辺の長い隣接領域へ統合。河川・湖沼・海は統合対象外。","保存タイル分割・地形LODとの連動はこの比較版の対象外。"]}
    dbpath=out/'landcover-vectors.sqlite3'
    with closing(sqlite3.connect(dbpath))as db:
        db.executescript("PRAGMA page_size=512;CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT) WITHOUT ROWID;CREATE TABLE features(level INTEGER,class INTEGER,geometry BLOB,PRIMARY KEY(level,class));")
        for level,tolerance,minimum in LEVELS:
            groups=[[]for _ in range(5)];checks={"gapAreaM2":0.,"overlapAreaM2":0.}
            print("LOD start",level,flush=True)
            for batch_index,(faces,codes,tile,neighbors) in enumerate(batches):
                pieces,part=simplify_level(faces,codes,tolerance,minimum,tile,neighbors)
                for code,g in enumerate(pieces):groups[code].append(g)
                for key in checks:checks[key]+=part[key]
                if batch_index%8==7:print("LOD",level,"batches",batch_index+1,"/ 64",flush=True)
            grouped=[shapely.union_all(pieces)for pieces in groups]
            features=[];vertices=0;regions=0;blob_bytes=0;areas={}
            for code,g in enumerate(grouped):
                local=affine_transform(g,[1,0,0,-1,-bounds[0],bounds[3]])
                vertices+=int(shapely.get_num_coordinates(local));regions+=len(list(all_polygons(local)))
                wkb=zlib.compress(shapely.to_wkb(local),9);blob_bytes+=len(wkb)
                db.execute("INSERT INTO features VALUES(?,?,?)",(level,code,wkb))
                features.append({"type":"Feature","properties":{"class":code},"geometry":mapping(local)})
                areas[str(code)]=float(g.area)
            raw=json.dumps({"type":"FeatureCollection","features":features},separators=(',',':')).encode()
            body=gzip.compress(raw,9,mtime=0);name=f'level-{level}.geojson.gz';(out/name).write_bytes(body)
            row={"level":level,"tolerance":tolerance,"minimumArea":minimum,"file":name,"transferBytes":len(body),"geojsonBytes":len(raw),"wkbZlibBytes":blob_bytes,"vertices":vertices,"polygons":regions,"areaM2":areas,**checks}
            manifest["levels"].append(row);print("LOD",level,"vertices",vertices,"gzip bytes",len(body),flush=True)
        manifest["relief"]=relief(bounds,out)
        manifest["seconds"]=time.monotonic()-start
        db.execute("INSERT INTO metadata VALUES('manifest',?)",(json.dumps(manifest,ensure_ascii=False),));db.commit()
        if db.execute("PRAGMA integrity_check").fetchone()[0]!="ok":raise ValueError("DB integrity")
    manifest["sqliteBytes"]=dbpath.stat().st_size
    (out/'manifest.json').write_text(json.dumps(manifest,ensure_ascii=False,indent=2),encoding='utf8')
    print("Complete",manifest["seconds"],"seconds",flush=True)
if __name__=="__main__":
    p=argparse.ArgumentParser();p.add_argument('--output',type=Path,default=ROOT/'build/landcover-vector-fuji-v1');p.add_argument('--size',type=int,default=4096)
    a=p.parse_args();generate(a.output,a.size)
