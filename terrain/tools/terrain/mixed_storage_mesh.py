"""同じ親TINを8 km・16 kmの保存タイルへ配置して混在接続を確認する。"""
import json,sqlite3,struct,zlib
from contextlib import closing
import numpy as np
from scipy.ndimage import map_coordinates
from national_independent_mesh import ROOT
from weighted_mesh_allocation import split_mesh
from compare_shared_allocation import encode_surface_mask
from uncut_mesh_topology import tile_topology,update_metrics


def generate(mode='error-detail20',error=25.,checkpoint_prefix=None,checkpoint_name='pruned.npz'):
    source=ROOT/f'build/terrain-shared-allocation-{mode}.sqlite3';output=ROOT/f'build/terrain-shared-allocation-{mode}-mixed.sqlite3'
    if output.exists():raise ValueError('既存の混在比較データは上書きしません')
    with closing(sqlite3.connect(source)) as src,closing(sqlite3.connect(output)) as db,db:
        src.backup(db);m=json.loads(db.execute('select value from metadata where key="manifest"').fetchone()[0]);m.update(coastlineInMesh=False,missingElevationPlaceholderMeters=0);m['sharedAllocation']['mode']=mode+'-mixed';m['sharedAllocation']['mixedStorage']=True;m['sharedAllocation']['generationComplete']=False
        db.execute('delete from tiles');db.execute('delete from topology')
        for level,x,y,body in db.execute('SELECT level,x,y,mask FROM tiles').fetchall():
            n=round(np.sqrt(len(zlib.decompress(body))*8));full=encode_surface_mask(np.ones((n,n),bool));db.execute('UPDATE tiles SET mask=? WHERE level=? AND x=? AND y=?',(full,level,x,y))
        for area in m['reliefRegions']:
            name=area['id'];old=area['levels'][0];ox=min(v[1] for v in old['keys']);oy=min(v[2] for v in old['keys']);keys=[]
            assert ox%2==0 and oy%2==0
            for key in old['keys']:
                if 6<=key[1]-ox<10 and 6<=key[2]-oy<10:
                    db.execute('insert into tiles values(?,?,?,?,?,?,?)',src.execute('select * from tiles where level=? and x=? and y=?',key).fetchone());db.execute('insert into topology values(?,?,?,?)',src.execute('select * from topology where level=? and x=? and y=?',key).fetchone());keys.append(key)
            with np.load(ROOT/f"build/{checkpoint_prefix or 'terrain-error20'}{'' if error==25 else '-e'+format(error,'g')}-{name}/{checkpoint_name}") as data:p=data['p'].copy();f=data['f']
            p[:,:2]*=31.25;ranks=np.arange(len(p))/max(1,len(p)-1);rankmap={tuple(v[:2]):r for v,r in zip(p,ranks)}
            with np.load(ROOT/f'build/terrain-quality-source-cache-v1/{name}.npz') as data:valid=data['valid'];gx=data['gx'];gy=data['gy']
            records=set()
            for (x,y),part in split_mesh(p,f,width=128000,tile_width=16000).items():
                if x in (3,4) and y in (3,4):continue
                used=np.unique(part['faces']);remap=np.zeros(len(part['points']),np.uint32);remap[used]=np.arange(len(used));v=part['points'][used];tri=remap[part['faces']];xy=v[:,:2]+[x*16000,y*16000];nv=len(v);assert nv<65536
                rank=np.asarray([rankmap.get(tuple(q),-1.) for q in xy],'<f4');body=zlib.compress(struct.pack('<4sHHII',b'YNT3',513,513,nv,tri.size)+v.astype('<f4').tobytes()+tri.astype('<u2').tobytes()+rank.tobytes(),9)
                uv=xy/31.25;slopes=np.c_[map_coordinates(gx,[uv[:,1],uv[:,0]],order=1),-map_coordinates(gy,[uv[:,1],uv[:,0]],order=1)];h=zlib.compress(struct.pack('<4sIf',b'YNH1',nv,8.)+np.rint(np.clip(slopes,-8,8)/8*32767).astype('<i2').tobytes(),9)
                key=[4,ox//2+x,oy//2+y];mask=encode_surface_mask(valid[y*512:(y+1)*512+1,x*512:(x+1)*512+1]);db.execute('insert into tiles values(?,?,?,?,?,?,?)',(*key,body,h,mask,json.dumps(dict(vertices=nv))));keys.append(key);records.add(tuple(key))
            for (x,y),body,count in tile_topology(p,f,width=16000,ranks=ranks,sample_count=513):
                key=(4,ox//2+x,oy//2+y)
                if key in records:db.execute('insert into topology values(?,?,?,?)',(*key,body))
            assert len(keys)==76;old['keys']=keys;old['tileCount']=76;print('混在保存完了',name,'8km16枚＋16km60枚',flush=True)
        m['sharedAllocation']['generationComplete']=True;update_metrics(db,m)
    return output

if __name__=='__main__':generate()
