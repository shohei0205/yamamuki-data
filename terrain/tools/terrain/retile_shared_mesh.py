"""既存の親TINを変えず、保存するタイルの一辺だけを変更する。"""
import argparse,json,sqlite3,struct,zlib
from contextlib import closing
import numpy as np
from scipy.ndimage import map_coordinates
from national_independent_mesh import ROOT
from weighted_mesh_allocation import split_mesh
from uncut_mesh_topology import restore,tile_topology,update_metrics
from compare_shared_allocation import encode_surface_mask
from verify_shared_allocation import verify


def source_mesh(db,area,width):
    points=[];faces=[];ranks=[];lookup={};keys=area['levels'][0]['keys'];ox=min(k[1] for k in keys);oy=min(k[2] for k in keys)
    for key in keys:
        raw=zlib.decompress(db.execute('SELECT geometry FROM tiles WHERE level=? AND x=? AND y=?',key).fetchone()[0]);magic,n,ny,nv,ni=struct.unpack_from('<4sHHII',raw);assert magic==b'YNT3'
        p=np.frombuffer(raw,'<f4',nv*3,16).reshape(-1,3).astype(float);p[:,:2]+=[(key[1]-ox)*width,(key[2]-oy)*width]
        f=np.frombuffer(raw,'<u2',ni,16+nv*12).reshape(-1,3);rank=np.frombuffer(raw,'<f4',nv,16+nv*12+ni*2);ids=[]
        for v,r in zip(p,rank):
            k=tuple(np.rint(v*[256,256,1024]).astype(int));i=lookup.get(k)
            if i is None:i=len(points);lookup[k]=i;points.append(v);ranks.append(float(r))
            else:ranks[i]=max(ranks[i],float(r))
            ids.append(i)
        faces.extend(np.asarray(ids)[f])
    p=np.asarray(points);f=np.asarray(faces,dtype=np.uint32);ranks=np.asarray(ranks)
    on_grid=np.max(np.abs(p[:,:2]/31.25-np.rint(p[:,:2]/31.25)),axis=1)<1e-6
    f,removed=restore(p,f,np.flatnonzero((ranks<0)&~on_grid))
    return p,f,ranks,ox,oy


def direct_source_mesh(db,area,width):
    keys=area['levels'][0]['keys'];ox=min(k[1] for k in keys);oy=min(k[2] for k in keys)
    points=[];ranks=[];lookup={};faces=set()
    for key in keys:
        raw=zlib.decompress(db.execute('SELECT data FROM topology WHERE level=? AND x=? AND y=?',key).fetchone()[0])
        magic,n,nv,ni,owned=struct.unpack_from('<4sIIII',raw);assert magic==b'YNU2'
        p=np.frombuffer(raw,'<f4',nv*3,20).reshape(-1,3).astype(float);p[:,:2]+=[(key[1]-ox)*width,(key[2]-oy)*width]
        f=np.frombuffer(raw,'<u4',ni,20+nv*12).reshape(-1,3);r=np.frombuffer(raw,'<f4',nv,20+nv*24+ni*4);ids=[]
        for v,rank in zip(p,r):
            k=tuple(np.rint(v*[256,256,1024]).astype(int));i=lookup.get(k)
            if i is None:i=len(points);lookup[k]=i;points.append(v);ranks.append(float(rank))
            ids.append(i)
        for tri in np.asarray(ids)[f]:
            j=int(np.argmin(tri));faces.add(tuple(np.roll(tri,-j)))
    return np.asarray(points),np.asarray(sorted(faces),np.uint32),np.asarray(ranks),ox,oy


def repair_serialized_faces(points,faces):
    # Float32の刻みが広がった際に反転する微小面だけ、隣の面と対角線を交換する。
    p=points.astype('<f4').astype(float);f=faces.copy()
    def cross(t):
        a,b,c=p[t,:2];u=b-a;v=c-a;return float(u[0]*v[1]-u[1]*v[0])
    for _ in range(20):
        bad=[i for i,t in enumerate(f) if cross(t)<=0]
        if not bad:return f
        for i in bad:
            for a,b in zip(f[i],np.roll(f[i],-1)):
                near=np.flatnonzero(np.any(f==a,axis=1)&np.any(f==b,axis=1));near=near[near!=i]
                if len(near)!=1:continue
                j=int(near[0]);c=next(v for v in f[i] if v not in (a,b));d=next(v for v in f[j] if v not in (a,b))
                first=np.array([c,d,b]);second=np.array([d,c,a])
                if cross(first)>0 and cross(second)>0 and abs(cross(first)+cross(second)-cross(f[i])-cross(f[j]))<1e-6:
                    f[i]=first;f[j]=second;break
            else:raise ValueError('Float32格納で反転した微小面を補修できません')
    raise ValueError('微小面の補修が収束しません')


def source_mask(db,area,width,spacing):
    n=round(128000/spacing)+1;mask=np.zeros((n,n),bool);seen=np.zeros((n,n),bool);keys=area['levels'][0]['keys'];ox=min(k[1] for k in keys);oy=min(k[2] for k in keys);samples=round(width/spacing)+1
    for key in keys:
        raw=zlib.decompress(db.execute('SELECT mask FROM tiles WHERE level=? AND x=? AND y=?',key).fetchone()[0]);bits=np.unpackbits(np.frombuffer(raw,np.uint8),bitorder='little')[:samples*samples].reshape(samples,samples)[::-1].astype(bool)
        x=(key[1]-ox)*(samples-1);y=(key[2]-oy)*(samples-1);part=mask[y:y+samples,x:x+samples];known=seen[y:y+samples,x:x+samples]
        assert np.array_equal(part[known],bits[known]);part[:]=bits;known[:]=True
    assert seen.all();return mask


def retile(mode,width):
    source=ROOT/f'build/terrain-shared-allocation-{mode}.sqlite3';pending_mode=mode+'-retile-pending';target=ROOT/f'build/terrain-shared-allocation-{pending_mode}.sqlite3';reports=[]
    with closing(sqlite3.connect(source)) as src,closing(sqlite3.connect(target)) as db,db:
        m=json.loads(src.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0]);m.update(coastlineInMesh=False,missingElevationPlaceholderMeters=0);old_width=m['sharedAllocation'].get('storageWidthMeters',8000);spacing=m['sharedAllocation'].get('evaluationSpacing',31.25);n=round(width/spacing)+1;level=round(np.log2(width/1000))
        if old_width==width:raise ValueError('指定した保存サイズへ変更済みです')
        src.backup(db);db.execute('DELETE FROM tiles');db.execute('DELETE FROM topology')
        m['levels']=[dict(m['levels'][0],level=level,widthMeters=width)];old_reports=m['sharedAllocation'].get('topologyReports',[])
        for area in m['reliefRegions']:
            print('親TINを復元',mode,area['id'],flush=True)
            old_report=next((v for v in old_reports if v['region']==area['id']),None)
            p,f,ranks,ox,oy=(direct_source_mesh if old_report and old_report.get('source')=='direct-parent-tin' else source_mesh)(src,area,old_width)
            if old_report:assert len(f)==old_report['facesAfter'], '元メッシュの面数が一致しません'
            ox2=ox*old_width/width;oy2=oy*old_width/width;assert ox2==int(ox2) and oy2==int(oy2);ox2=int(ox2);oy2=int(oy2)
            mask=source_mask(src,area,old_width,spacing)
            with np.load(ROOT/f"build/terrain-quality-source-cache-v1/{area['id']}.npz") as data:gx=data['gx'];gy=data['gy']
            old_level=area['levels'][0];q=old_level['priorityMetrics'];new_keys=[];stored=0;geometry_bytes=0
            rankmap={tuple(np.rint(v[:2]*256).astype(int)):float(r) for v,r in zip(p,ranks)}
            for (x,y),part in split_mesh(p,f,width=128000,tile_width=width).items():
                used=np.unique(part['faces']);remap=np.zeros(len(part['points']),np.uint32);remap[used]=np.arange(len(used));v=part['points'][used];t=remap[part['faces']];xy=v[:,:2]+[x*width,y*width];nv=len(v);assert nv<65536
                t=repair_serialized_faces(v,t)
                r=np.array([rankmap.get(tuple(np.rint(a*256).astype(int)),-1.) for a in xy],dtype='<f4');r[np.any((v[:,:2]==0)|(v[:,:2]==width),axis=1)]=-1
                g=zlib.compress(struct.pack('<4sHHII',b'YNT3',n,n,nv,t.size)+v.astype('<f4').tobytes()+t.astype('<u2').tobytes()+r.tobytes(),9)
                uv=xy/spacing;slopes=np.column_stack([map_coordinates(gx,[uv[:,1],uv[:,0]],order=1),-map_coordinates(gy,[uv[:,1],uv[:,0]],order=1)])
                h=zlib.compress(struct.pack('<4sIf',b'YNH1',nv,8.)+np.rint(np.clip(slopes,-8,8)/8*32767).astype('<i2').tobytes(),9)
                bits=encode_surface_mask(mask[y*(n-1):y*(n-1)+n,x*(n-1):x*(n-1)+n]);key=[level,ox2+x,oy2+y];new_keys.append(key)
                db.execute('INSERT INTO tiles VALUES(?,?,?,?,?,?,?)',(*key,g,h,bits,json.dumps(dict(vertices=nv))));stored+=nv;geometry_bytes+=len(g)
            expected=round(128000/width)**2;assert len(new_keys)==expected;records={tuple(k) for k in new_keys};written=0
            for (x,y),body,count in tile_topology(p,f,width=width,ranks=ranks,sample_count=n):
                key=(level,ox2+x,oy2+y)
                if key not in records:continue
                db.execute('INSERT INTO topology VALUES(?,?,?,?)',(*key,body));written+=1
            assert written==expected
            area['levels']=[dict(old_level,level=level,widthMeters=width,keys=new_keys,tileCount=len(new_keys))]
            q.update(vertices=stored,geometryBytes=geometry_bytes,clippedVertices=stored,clippedGeometryBytes=geometry_bytes)
            report=dict(region=area['id'],sourceVertices=q['sourceVertices'],facesBefore=len(f),facesAfter=len(f),tiles=written,artificialCandidates=0,removed=0,source='original-parent-tin-retiled');reports.append(report);print(report,flush=True)
        a=m['sharedAllocation'];a['storageWidthMeters']=width;a['previousStorageWidthMeters']=old_width;a['topologyReports']=reports;a['retiledWithoutSimplification']=True
        update_metrics(db,m)
    result=verify(pending_mode);print('タイル検証完了',mode,width,result['tiles'],result['sharedEdges'],flush=True)
    return dict(mode=mode,widthMeters=width,verification=result,regions=reports,pending=str(target),original=str(source))


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--mode',required=True);parser.add_argument('--width-km',type=int,choices=[16,32,64,128],required=True);args=parser.parse_args()
    result=retile(args.mode,args.width_km*1000);(ROOT/f'build/terrain-retile-{args.width_km}-{args.mode}.json').write_text(json.dumps(result,ensure_ascii=False,indent=2),encoding='utf-8')
