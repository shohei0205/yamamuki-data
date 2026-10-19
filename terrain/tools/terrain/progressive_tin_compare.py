"""同じTINの各段階を独立LOD・共有頂点・挿入履歴で比較保存する。"""
import hashlib,json,sqlite3,struct,subprocess,time,zlib
from pathlib import Path
import numpy as np
from PIL import Image
from scipy.ndimage import gaussian_filter,map_coordinates
from fixed_mesh_preview import ROOT,source_grid
from vertex_normals import encode_oct
BUDGETS=[512,1024,2048]
BOUNDARY_ERROR=25
REGIONS={'fuji':('富士山',35.3606,138.7274),'yari':('槍ヶ岳',36.3419,137.6476),'plain':('関東平野',36.05,139.65)}
OUTPUT=ROOT/'build/progressive-tin-compare.sqlite3'

def uint(v):
    out=bytearray()
    while v>=128:out.append((v&127)|128);v>>=7
    out.append(v);return out

def face(f):return uint(f['id'])+struct.pack('<3H',*f['vertices'])
def vertices_blob(points,normals,first,last):
    raw=bytearray(struct.pack('<4sII',b'PVX1',first,last-first))
    for p,n in zip(points[first:last],normals[first:last]):raw.extend(struct.pack('<HHiBB',int(p[0]),int(p[1]),round(p[2]*4),*n))
    return bytes(raw)
def index_blob(stage):return struct.pack('<4sII',b'PIX1',stage['vertexCount'],len(stage['faces']))+b''.join(struct.pack('<3H',*f['vertices']) for f in stage['faces'])
def base_blob(base):return struct.pack('<4sII',b'PBA1',base['vertexCount'],len(base['faces']))+b''.join(face(f) for f in base['faces'])
def delta_blob(steps,first,last,base_vertices):
    raw=bytearray(struct.pack('<4sII',b'PDI2',first,last-first))
    for offset,step in enumerate(steps[first:last],first):
        raw.extend(uint(len(step['removed'])))
        previous=0
        for i in step['removed']:raw.extend(uint(i-previous));previous=i
        raw.extend(uint(len(step['added'])))
        # 追加頂点番号と面IDは再生順から復元し、重複して保存しない。
        for f in step['added']:
            assert f['vertices'][2]==base_vertices+offset
            raw.extend(struct.pack('<2H',*f['vertices'][:2]))
    return bytes(raw)

def generate():
    manifest={'schemaVersion':1,'kind':'progressive-tin-comparison','extent':16000,'gridSize':685,'budgets':BUDGETS,'boundaryError':BOUNDARY_ERROR,'regions':{},'notes':'生成用DEMは配布しない。共通境界を固定した内部挿入の比較。標準PMのvertex split形式ではない。頂点上限を比較条件に固定し、内部誤差での早期停止は完全な平面の場合のみ。'}
    binary=ROOT/'build/terrain-rust-target/release/terrain-adaptive-tin.exe'
    with sqlite3.connect(OUTPUT) as db,sqlite3.connect((ROOT/'build/terrain-lod.sqlite3').resolve().as_uri()+'?mode=ro',uri=True) as src:
        db.execute('CREATE TABLE IF NOT EXISTS blobs(region TEXT,format TEXT,level INTEGER,kind TEXT,data BLOB,PRIMARY KEY(region,format,level,kind))');db.execute('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT)')
        for region,(name,lat,lon) in REGIONS.items():
            grid,_=source_grid(src,lat,lon,extent=16000,size=685);offset=min(0,float(np.floor(grid.min())));pixels=np.rint((grid-offset)*4)
            assert pixels.min()>=0 and pixels.max()<=65535
            suffix='-reference' if BUDGETS==[16384] else '-coarse' if BUDGETS==[128,256] else ''
            png=ROOT/f'build/progressive-{region}{suffix}.png';report=ROOT/f'build/progressive-{region}{suffix}.json';Image.fromarray(pixels.astype(np.uint16)).save(png)
            run=subprocess.run([str(binary),'--input',str(png),'--output',str(report),'--scale','.25','--offset',str(offset),'--progressive-vertices',','.join(map(str,BUDGETS)),'--boundary-error',str(BOUNDARY_ERROR)],check=True,capture_output=True,text=True,encoding='utf-8')
            mesh=json.loads(report.read_text(encoding='utf-8'));points=np.asarray(mesh['vertices']);dy,dx=np.gradient(gaussian_filter(grid.astype(float),1),16000/684);x,y=points[:,:2].T
            normal=np.column_stack((-map_coordinates(dx,[y,x],order=1),map_coordinates(dy,[y,x],order=1),np.ones(len(points))));normal/=np.linalg.norm(normal,axis=1,keepdims=True);normals=encode_oct(normal)
            totals={f:0 for f in ['independent','shared','history']};stages=[];first_vertex=0;first_step=0
            def save(fmt,level,kind,raw):
                body=zlib.compress(raw,9);db.execute('INSERT OR REPLACE INTO blobs VALUES(?,?,?,?,?)',(region,fmt,level,kind,body));totals[fmt]+=len(body);return len(body)
            base_bytes=save('history',-1,'base',base_blob(mesh['base']))
            faces={f['id']:f['vertices'] for f in mesh['base']['faces']};boundary={i for i,p in enumerate(points) if p[0] in (0,684) or p[1] in (0,684)};assert all(i<mesh['base']['vertexCount'] for i in boundary)
            for level,stage in enumerate(mesh['stages']):
                for step in mesh['steps'][first_step:stage['stepCount']]:
                    for id in step['removed']:assert faces.pop(id,None) is not None
                    for f in step['added']:assert f['id'] not in faces;faces[f['id']]=f['vertices']
                assert faces=={f['id']:f['vertices'] for f in stage['faces']}
                index=index_blob(stage);nv=stage['vertexCount'];positions=vertices_blob(points,normals,0,nv);append=vertices_blob(points,normals,first_vertex,nv)
                independent=save('independent',level,'vertices',positions)+save('independent',level,'indices',index)
                shared=save('shared',level,'vertices',append)+save('shared',level,'indices',index)
                history=save('history',level,'vertices',append)+save('history',level,'delta',delta_blob(mesh['steps'],first_step,stage['stepCount'],mesh['base']['vertexCount']))
                stages.append(dict(budget=BUDGETS[level],vertices=nv,triangles=len(stage['faces']),maxError=stage['maxError'],independentBytes=independent,sharedBytes=shared,historyBytes=history,historyBaseBytes=base_bytes if level==0 else 0,cumulative=dict(totals),indicesSha256=hashlib.sha256(index).hexdigest()))
                first_vertex=nv;first_step=stage['stepCount']
            manifest['regions'][region]=dict(name=name,lat=lat,lon=lon,baseVertices=mesh['base']['vertexCount'],stages=stages,totalBytes=totals,generator=json.loads(run.stdout));print(region,json.dumps(manifest['regions'][region]),flush=True)
        db.execute('INSERT OR REPLACE INTO metadata VALUES(?,?)',('manifest',json.dumps(manifest,ensure_ascii=False)));db.commit()
    (OUTPUT.with_suffix('.json') if BUDGETS!=[512,1024,2048] else ROOT/'build/progressive-tin-comparison.json').write_text(json.dumps(manifest,ensure_ascii=False,indent=2),encoding='utf-8')
if __name__=='__main__':generate()
