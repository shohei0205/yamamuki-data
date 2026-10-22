"""指定地域だけを角度制約付きTINに置き換え、元版は別ファイルで保持する。"""
import json,sqlite3,struct,zlib
from contextlib import closing
import numpy as np
from scipy.ndimage import map_coordinates
from compare_shared_allocation import ROOT,encode_mask,residual
from weighted_mesh_allocation import split_mesh,movable_priority
from uncut_mesh_topology import tile_topology,update_metrics
from verify_shared_allocation import verify


def replace_region(source_mode,mode,region_id,points,faces,quality):
    source=ROOT/f'build/terrain-shared-allocation-{source_mode}.sqlite3';pending_mode=mode+'-pending';pending=ROOT/f'build/terrain-shared-allocation-{pending_mode}.sqlite3';target=ROOT/f'build/terrain-shared-allocation-{mode}.sqlite3'
    with np.load(ROOT/f'build/terrain-quality-source-cache-v1/{region_id}.npz') as data:fields={key:data[key] for key in ['grid','valid','gx','gy','shape_limits','limits']}
    with closing(sqlite3.connect(source)) as src,closing(sqlite3.connect(pending)) as db,db:
        src.backup(db);m=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0]);a=m['sharedAllocation'];area=next(v for v in m['reliefRegions'] if v['id']==region_id);level=area['levels'][0];keys=level['keys'];records={tuple(k) for k in keys};ox=min(k[1] for k in keys);oy=min(k[2] for k in keys)
        error=residual(points,faces,fields['grid']);e=error[fields['valid']];quality.update(rmse=float(np.sqrt(np.mean(e*e))),maximum=float(np.max(np.abs(e))))
        world=points.copy();world[:,:2]*=31.25;ranks=np.arange(len(points),dtype=float)/max(1,len(points)-1);rankmap={tuple(v[:2]):float(r) for v,r in zip(world,ranks)};stored=0;geometry_bytes=0
        for (tx,ty),part in split_mesh(world,faces,width=128000.).items():
            key=(3,ox+tx,oy+ty);assert key in records
            used=np.unique(part['faces']);remap=np.zeros(len(part['points']),dtype=np.uint32);remap[used]=np.arange(len(used));p=part['points'][used];f=remap[part['faces']];xy=p[:,:2]+[tx*8000,ty*8000];rank=np.array([rankmap.get(tuple(v),-1.) for v in xy],dtype='<f4');rank[np.any((p[:,:2]==0)|(p[:,:2]==8000),axis=1)]=-1
            nv=len(p);assert nv<65536
            g=zlib.compress(struct.pack('<4sHHII',b'YNT3',257,257,nv,f.size)+p.astype('<f4').tobytes()+f.astype('<u2').tobytes()+rank.tobytes(),9)
            uv=xy/31.25;slopes=np.column_stack([map_coordinates(fields['gx'],[uv[:,1],uv[:,0]],order=1),-map_coordinates(fields['gy'],[uv[:,1],uv[:,0]],order=1)])
            h=zlib.compress(struct.pack('<4sIf',b'YNH1',nv,8.)+np.rint(np.clip(slopes,-8,8)/8*32767).astype('<i2').tobytes(),9)
            mask=encode_mask(fields['valid'][ty*256:(ty+1)*256+1,tx*256:(tx+1)*256+1])
            db.execute('UPDATE tiles SET geometry=?,hints=?,mask=?,stats=? WHERE level=? AND x=? AND y=?',(g,h,mask,json.dumps(dict(vertices=nv)),*key));stored+=nv;geometry_bytes+=len(g)
        # 親TINから直接余白付き接続を作り、切断点の復元による細長い面も発生させない。
        written=0
        for (tx,ty),body,count in tile_topology(world,faces,ranks=ranks):
            key=(3,ox+tx,oy+ty)
            if key not in records:continue
            db.execute('INSERT OR REPLACE INTO topology VALUES(?,?,?,?)',(*key,body));written+=1
        assert written==len(keys)
        q=level['priorityMetrics'];q.update(sourceVertices=len(points),vertices=stored,geometryBytes=geometry_bytes,clippedVertices=stored,clippedGeometryBytes=geometry_bytes,maximum=quality['maximum'],rmse=quality['rmse'],weightedMaximum=float(np.max(np.abs(e)/fields['limits'][fields['valid']])) ,allocationScore=movable_priority(error,fields['shape_limits'],fields['valid']))
        a['mode']=mode;a['sourceVertices']=sum(v['levels'][0]['priorityMetrics']['sourceVertices'] for v in m['reliefRegions']);a['meshQuality']['regions'][region_id]=quality;a['meshQuality']['angleGuardRegion']=region_id
        a['topologyReports']=[v for v in a.get('topologyReports',[]) if v['region']!=region_id]+[dict(region=region_id,source='direct-parent-tin',tiles=written,artificialCandidates=0,removed=0,facesBefore=len(faces),facesAfter=len(faces))]
        update_metrics(db,m)
    result=verify(pending_mode);pending.replace(target)
    print('地域の置き換えを検証',result['tiles'],result['sharedEdges'],quality,flush=True)
    return result


if __name__=='__main__':
    import argparse
    from generate_shared_quality_budgets import read_parent,budget_parent
    from graded_budget_mesh import graded_mesh
    parser=argparse.ArgumentParser();parser.add_argument('--source-mode',default='landform-quality128-b12000');parser.add_argument('--output-mode',default='landform-quality128-b12000-graded');parser.add_argument('--region',default='tsukuba');args=parser.parse_args()
    if args.source_mode==args.output_mode:parser.error('元データと別の出力を指定してください')
    with closing(sqlite3.connect(ROOT/f'build/terrain-shared-allocation-{args.source_mode}.sqlite3')) as db:
        m=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0]);area=next(a for a in m['reliefRegions'] if a['id']==args.region);budget=area['levels'][0]['priorityMetrics']['sourceVertices'];total=m['sharedAllocation']['totalBudget'];old=m['sharedAllocation']['meshQuality']['regions'][args.region]
    with closing(sqlite3.connect(ROOT/'build/terrain-shared-allocation-landform128.sqlite3')) as db:
        base=json.loads(db.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0]);p,ranks=read_parent(db,next(a for a in base['reliefRegions'] if a['id']==args.region))
    with np.load(ROOT/f'build/terrain-quality-source-cache-v1/{args.region}.npz') as data:grid=data['grid'];limits=data['shape_limits']
    p,ranks,_,tolerance=budget_parent(p,ranks,grid,total);points,faces,quality=graded_mesh(p,ranks,grid,budget,limits=limits)
    quality.update(beforeVertices=budget,beforeUnder10=old['under10'],beforeUnder5=old['under5'],budget=budget,boundaryToleranceMeters=tolerance)
    replace_region(args.source_mode,args.output_mode,args.region,points,faces,quality)
