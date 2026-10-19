"""独立TINの頂点を動かし、誤差と稜線・シルエットの変化を比較する試作。"""
import json,sqlite3,struct,time,zlib
from pathlib import Path
import numpy as np
from scipy import sparse
from scipy.sparse.linalg import lsmr
from scipy.optimize import linprog
from scipy.ndimage import gaussian_filter,map_coordinates
from fixed_mesh_preview import ROOT,source_grid
from progressive_tin_compare import REGIONS
from vertex_normals import encode_oct,decode_oct
VARIANTS={'z_rms':'標高のみ・RMSE','z_max':'標高のみ・最大誤差','xy_rms':'XY＋標高・RMSE','xy_max':'XY＋標高・最大誤差','feature_z':'標高のみ・山頂／稜線／輪郭重視','feature_xy':'XY＋標高・山頂／稜線／輪郭重視','volume_z':'標高のみ・絶対体積差','volume_xy':'XY＋標高・絶対体積差'}
OUTPUT=ROOT/'build/tin-optimized-compare.sqlite3'

def raster(points,faces,size):
    owner=np.full(size*size,-1,np.int32);weights=np.zeros((size*size,3))
    for i,ids in enumerate(faces):
        tri=points[ids];a,b,c=tri[:,:2];den=(b[0]-a[0])*(c[1]-a[1])-(b[1]-a[1])*(c[0]-a[0])
        if den<=1e-7:raise ValueError('反転または退化面')
        xmin=max(0,int(np.ceil(tri[:,0].min()-1e-7)));xmax=min(size-1,int(np.floor(tri[:,0].max()+1e-7)));ymin=max(0,int(np.ceil(tri[:,1].min()-1e-7)));ymax=min(size-1,int(np.floor(tri[:,1].max()+1e-7)))
        if xmin>xmax or ymin>ymax:continue
        yy,xx=np.mgrid[ymin:ymax+1,xmin:xmax+1];u=((xx-a[0])*(c[1]-a[1])-(yy-a[1])*(c[0]-a[0]))/den;v=((b[0]-a[0])*(yy-a[1])-(b[1]-a[1])*(xx-a[0]))/den;inside=(u>=-1e-8)&(v>=-1e-8)&(u+v<=1+1e-8);loc=(yy[inside]*size+xx[inside]);owner[loc]=i;weights[loc]=np.column_stack((1-u[inside]-v[inside],u[inside],v[inside]))
    if np.any(owner<0):raise ValueError('未被覆の格子点')
    return owner,weights

def predicted(points,faces,owner,weights):return (points[faces[owner],2]*weights).sum(axis=1)
def matrix(points,faces,owner,weights,free,xy):
    remap=np.full(len(points),-1);remap[free]=np.arange(len(free));ids=faces[owner];rows=np.repeat(np.arange(len(owner)),3);cols=remap[ids].ravel();values=weights.ravel();keep=cols>=0
    if not xy:return sparse.csr_matrix((values[keep],(rows[keep],cols[keep])),shape=(len(owner),len(free)))
    tri=points[faces];a,b,c=tri[:,0],tri[:,1],tri[:,2];den=(b[:,0]-a[:,0])*(c[:,1]-a[:,1])-(b[:,1]-a[:,1])*(c[:,0]-a[:,0]);gx=((b[:,2]-a[:,2])*(c[:,1]-a[:,1])-(c[:,2]-a[:,2])*(b[:,1]-a[:,1]))/den;gy=((b[:,0]-a[:,0])*(c[:,2]-a[:,2])-(c[:,0]-a[:,0])*(b[:,2]-a[:,2]))/den
    parts=[]
    for axis,factor in enumerate([-gx[owner],-gy[owner],np.ones(len(owner))]):parts.append(sparse.csr_matrix(((weights*factor[:,None]).ravel()[keep],(rows[keep],cols[keep]*3+axis)),shape=(len(owner),len(free)*3)))
    return sum(parts)

def extrema(owner,residual):
    order=np.lexsort((residual,owner));starts=np.r_[0,np.flatnonzero(np.diff(owner[order]))+1];ends=np.r_[starts[1:]-1,len(order)-1];return np.unique(np.r_[order[starts],order[ends]])
def minimax(jac,residual,owner,bounds,rounds=5):
    active=extrema(owner,residual);delta=np.zeros(jac.shape[1]);target=float(np.abs(residual).max());converged=False
    for _ in range(rounds):
        a=jac[active];one=sparse.csr_matrix(-np.ones((len(active),1)));constraints=sparse.vstack((sparse.hstack((a,one)),sparse.hstack((-a,one))),format='csr');rhs=np.r_[-residual[active],residual[active]]
        result=linprog(np.r_[np.zeros(jac.shape[1]),1.],A_ub=constraints,b_ub=rhs,bounds=bounds+[(0,None)],method='highs',options={'time_limit':20})
        if not result.success:break
        delta=result.x[:-1];target=float(result.x[-1]);new=residual+jac@delta
        if np.abs(new).max()<=target+.02:converged=True;break
        active=np.union1d(active,extrema(owner,new))
    return delta,target,converged

def features(grid):
    size=len(grid);yy,xx=np.indices(grid.shape);flat=np.c_[xx.ravel(),yy.ravel()];truth=grid.ravel();views=[];mask=np.zeros(grid.size,bool)
    for angle in np.arange(8)*np.pi/4:
        camera=np.array([size/2+np.sin(angle)*size*1.5,size/2+np.cos(angle)*size*1.5]);delta=flat-camera;az=np.arctan2(delta[:,0],delta[:,1]);centre=np.arctan2(size/2-camera[0],size/2-camera[1]);relative=(az-centre+np.pi)%(2*np.pi)-np.pi;bins=np.clip(((relative+.45)/.9*128).astype(int),0,127);distance=np.linalg.norm(delta,axis=1)*(16000/(size-1));height=np.arctan2(truth-1000,distance);peaks=np.full(128,-np.inf);np.maximum.at(peaks,bins,height);mask|=height>=peaks[bins]-np.deg2rad(.02);views.append((bins,distance,peaks))
    curvature=np.abs(grid-gaussian_filter(grid,3)).ravel();weight=1+4*np.clip(curvature/max(np.percentile(curvature,90),1),0,2)+mask*30
    return weight,views

def metrics(points,faces,grid,feature_weights,views):
    owner,w=raster(points,faces,len(grid));estimate=predicted(points,faces,owner,w);error=estimate-grid.ravel();sil=[]
    for bins,distance,truth in views:
        curve=np.full(128,-np.inf);np.maximum.at(curve,bins,np.arctan2(estimate-1000,distance));valid=np.isfinite(curve)&np.isfinite(truth);sil.extend(np.rad2deg(curve[valid]-truth[valid]))
    integration=np.ones(grid.shape);integration[[0,-1],:]*=.5;integration[:,[0,-1]]*=.5;absolute_volume=float(np.sum(np.abs(error)*integration.ravel())*(16000/(len(grid)-1))**2)
    return dict(absoluteVolumeM3=absolute_volume,mae=float(np.mean(np.abs(error))),maxError=float(np.abs(error).max()),rmse=float(np.sqrt(np.mean(error**2))),featureRmse=float(np.sqrt(np.average(error**2,weights=feature_weights))),silhouetteMaxDegrees=float(np.abs(sil).max()),silhouetteRmseDegrees=float(np.sqrt(np.mean(np.array(sil)**2)))),owner,w

def locked_vertices(points,faces,size):
    locked=(points[:,0]==0)|(points[:,1]==0)|(points[:,0]==size-1)|(points[:,1]==size-1);peak=np.ones(len(points),bool)
    for f in faces:
        for v in f:
            if points[v,2]<points[f,2].max():peak[v]=False
    chosen=[]
    for i in np.argsort(-points[:,2]):
        if peak[i] and not locked[i] and all(np.linalg.norm(points[i,:2]-points[j,:2])>20 for j in chosen):chosen.append(i)
        if len(chosen)==8:break
    locked[chosen]=True;return locked,chosen

def optimize(original,faces,grid,variant,weights,views,initial=None):
    points=original.copy() if initial is None else initial.copy();xy=variant.startswith('xy_') or variant in ('feature_xy','volume_xy');volume=variant.startswith('volume_');maximum=variant.endswith('_max');feature=variant.startswith('feature_');locked,peaks=locked_vertices(original,faces,len(grid));points[locked]=original[locked];free=np.flatnonzero(~locked);base,owner,w=metrics(points,faces,grid,weights,views);accepted=0;converged=False
    if not len(free):return points,base,dict(iterations=0,peaks=peaks,converged=True)
    for iteration in range(8 if volume else (5 if xy else (1 if maximum else 2))):
        residual=predicted(points,faces,owner,w)-grid.ravel();jac=matrix(points,faces,owner,w,free,xy)
        if maximum:
            bounds=[(-1.5,1.5),(-1.5,1.5),(-150,150)]*len(free) if xy else [(-2000,2000)]*len(free)
            delta,target,converged=minimax(jac,residual,owner,bounds)
        else:
            # 規則サンプルに輪郭点を追加。受理判定と最終検証は元格子の全点で行う。
            ids=np.arange(grid.size).reshape(grid.shape)[::3,::3].ravel();ids=np.union1d(ids,np.flatnonzero(weights>25)) if feature else ids
            weight=np.sqrt(weights[ids]) if feature else (1/np.sqrt(np.maximum(np.abs(residual[ids]),.1)) if volume else np.ones(len(ids)));j=jac[ids].multiply(weight[:,None]);rhs=-residual[ids]*weight
            delta=lsmr(j,rhs,damp=.05,atol=1e-6,btol=1e-6,maxiter=120)[0]
            if xy:delta.reshape(-1,3)[:,:2]=np.clip(delta.reshape(-1,3)[:,:2],-1.5,1.5)
        change=np.zeros_like(points)
        if xy:change[free]=delta.reshape(-1,3)
        else:change[free,2]=delta
        key='absoluteVolumeM3' if volume else ('maxError' if maximum else ('featureRmse' if feature else 'rmse'));best=None
        for amount in [1.,.5,.25,.125,.0625]:
            candidate=points+change*amount
            if candidate[:,:2].min()<-1e-8 or candidate[:,:2].max()>len(grid)-1+1e-8:continue
            try:report,new_owner,new_w=metrics(candidate,faces,grid,weights,views)
            except ValueError:continue
            if report[key]<base[key]-1e-5 and (best is None or report[key]<best[1][key]):best=(candidate,report,new_owner,new_w)
        if best is None:break
        points,base,owner,w=best;accepted+=1
    assert np.array_equal(points[locked],original[locked]);return points,base,dict(iterations=accepted,peaks=[int(i) for i in peaks],converged=bool(converged),lockedVertices=int(locked.sum()),maxMoveMeters=float(np.linalg.norm((points[:,:2]-original[:,:2])*16000/(len(grid)-1),axis=1).max()))

def generate(selected=None):
    manifest={'schemaVersion':1,'extent':16000,'gridSize':685,'variants':VARIANTS,'regions':{}}
    with sqlite3.connect(OUTPUT) as db,sqlite3.connect((ROOT/'build/terrain-lod.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as src:
        db.execute('CREATE TABLE IF NOT EXISTS meshes(region TEXT,variant TEXT,level INTEGER,data BLOB,PRIMARY KEY(region,variant,level))');db.execute('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT)')
        existing=db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()
        if existing:manifest=json.loads(existing[0]);manifest['variants']=VARIANTS
        for region,(name,lat,lon) in REGIONS.items():
            grid,_=source_grid(src,lat,lon,extent=16000,size=685);grid=np.rint(grid*4)/4;weights,views=features(grid);mesh=json.loads((ROOT/f'build/progressive-{region}.json').read_text(encoding='utf-8'));records=[];dy,dx=np.gradient(gaussian_filter(grid.astype(float),1),16000/684)
            for level,stage in enumerate(mesh['stages']):
                original=np.asarray(mesh['vertices'][:stage['vertexCount']]);faces=np.array([f['vertices'] for f in stage['faces']],dtype=np.uint16);baseline,_,_=metrics(original,faces,grid,weights,views);variants=dict(manifest.get('regions',{}).get(region,{}).get('levels',[{} for _ in range(3)])[level].get('variants',{}))
                for variant in (selected or list(VARIANTS)):
                    start=time.perf_counter();seed=None;seed_variant=None
                    if variant.startswith('volume_'):
                        allowed=['original','z_rms','z_max','feature_z','volume_z'] if variant=='volume_z' else list(variants)
                        best_volume=metrics(original,faces,grid,weights,views)[0]['absoluteVolumeM3']
                        for candidate in allowed:
                            row=db.execute('SELECT data FROM meshes WHERE region=? AND variant=? AND level=?',(region,candidate,level)).fetchone()
                            if not row:continue
                            raw=zlib.decompress(row[0]);candidate_points=np.frombuffer(raw,'<f4',len(original)*3,12).reshape(-1,3).astype(float);candidate_points[:,0]=(candidate_points[:,0]/16000+.5)*684;candidate_points[:,1]=(.5-candidate_points[:,1]/16000)*684
                            if variant=='volume_z':candidate_points[:,:2]=original[:,:2]
                            locked,_=locked_vertices(original,faces,len(grid));candidate_points[locked]=original[locked];volume=metrics(candidate_points,faces,grid,weights,views)[0]['absoluteVolumeM3']
                            if volume<best_volume:best_volume=volume;seed=candidate_points;seed_variant=candidate
                    points,report,details=optimize(original,faces,grid,variant,weights,views,initial=seed);details['initialVariant']=seed_variant;world=points.copy();world[:,0]=(points[:,0]/684-.5)*16000;world[:,1]=(.5-points[:,1]/684)*16000;world=world.astype('<f4');decoded=world.astype(float);decoded[:,0]=(decoded[:,0]/16000+.5)*684;decoded[:,1]=(.5-decoded[:,1]/16000)*684;report,_,_=metrics(decoded,faces,grid,weights,views)
                    normals=np.c_[-map_coordinates(dx,[points[:,1],points[:,0]],order=1),map_coordinates(dy,[points[:,1],points[:,0]],order=1),np.ones(len(points))];normals/=np.linalg.norm(normals,axis=1,keepdims=True);octets=encode_oct(normals)
                    raw=struct.pack('<4sII',b'YOP1',len(points),faces.size)+world.tobytes()+faces.astype('<u2').tobytes()+octets.tobytes();body=zlib.compress(raw,9);db.execute('INSERT OR REPLACE INTO meshes VALUES(?,?,?,?)',(region,variant,level,body));variants[variant]=dict(report,**details,bytes=len(body),vertices=len(points),triangles=len(faces),seconds=time.perf_counter()-start);print(region,level,variant,json.dumps(variants[variant]),flush=True)
                    (ROOT/'build/tin-optimize.progress.json').write_text(json.dumps(dict(region=region,level=level,variant=variant)),encoding='utf-8')
                records.append(dict(baseline=baseline,variants=variants));db.commit()
            manifest['regions'][region]=dict(name=name,levels=records)
        db.execute('INSERT OR REPLACE INTO metadata VALUES(?,?)',('manifest',json.dumps(manifest,ensure_ascii=False)));db.commit()
    (ROOT/'build/tin-optimized-report.json').write_text(json.dumps(manifest,ensure_ascii=False,indent=2),encoding='utf-8')
def finalize():
    """全案を同じFloat32形式と全格子の評価指標でそろえる。"""
    with sqlite3.connect(OUTPUT) as db,sqlite3.connect((ROOT/'build/terrain-lod.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as src:
        manifest=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0])
        manifest['variants']={'original':'最適化なし（同じFloat32形式）',**{k:v for k,v in manifest['variants'].items() if k!='original'}}
        for region,(name,lat,lon) in REGIONS.items():
            grid,_=source_grid(src,lat,lon,extent=16000,size=685);grid=np.rint(grid*4)/4;weights,views=features(grid);dy,dx=np.gradient(gaussian_filter(grid.astype(float),1),16000/684);mesh=json.loads((ROOT/f'build/progressive-{region}.json').read_text(encoding='utf-8'))
            for level,stage in enumerate(mesh['stages']):
                points=np.asarray(mesh['vertices'][:stage['vertexCount']]);faces=np.asarray([f['vertices'] for f in stage['faces']],dtype='<u2');world=points.copy();world[:,0]=(world[:,0]/684-.5)*16000;world[:,1]=(.5-world[:,1]/684)*16000;world=world.astype('<f4');normal=np.c_[-map_coordinates(dx,[points[:,1],points[:,0]],order=1),map_coordinates(dy,[points[:,1],points[:,0]],order=1),np.ones(len(points))];normal/=np.linalg.norm(normal,axis=1,keepdims=True);body=zlib.compress(struct.pack('<4sII',b'YOP1',len(points),faces.size)+world.tobytes()+faces.tobytes()+encode_oct(normal).tobytes(),9);db.execute('INSERT OR REPLACE INTO meshes VALUES(?,?,?,?)',(region,'original',level,body));variants=manifest['regions'][region]['levels'][level]['variants'];variants['original']=dict(bytes=len(body),vertices=len(points),triangles=len(faces),iterations=0,seconds=0,maxMoveMeters=0,peaks=[],converged=True)
                for variant,record in variants.items():
                    data=zlib.decompress(db.execute('SELECT data FROM meshes WHERE region=? AND variant=? AND level=?',(region,variant,level)).fetchone()[0]);nv,ni=struct.unpack_from('<II',data,4);p=np.frombuffer(data,'<f4',nv*3,12).reshape(-1,3).astype(float);p[:,0]=(p[:,0]/16000+.5)*684;p[:,1]=(.5-p[:,1]/16000)*684;report,_,_=metrics(p,faces,grid,weights,views);record.update(report)
                print('評価を統一',region,level,flush=True)
        db.execute('UPDATE metadata SET value=? WHERE key="manifest"',(json.dumps(manifest,ensure_ascii=False),));db.commit()
    (ROOT/'build/tin-optimized-report.json').write_text(json.dumps(manifest,ensure_ascii=False,indent=2),encoding='utf-8')

if __name__=='__main__':
    import argparse
    parser=argparse.ArgumentParser();parser.add_argument('--variants',nargs='+',choices=list(VARIANTS));args=parser.parse_args();generate(args.variants);finalize()
