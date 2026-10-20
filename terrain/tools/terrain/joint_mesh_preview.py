"""富士山 L3 の比較用に、32 km を一度に Adaptive TIN 化して区画へ保存する。"""
import argparse,json,sqlite3,struct,subprocess,tempfile,zlib
from pathlib import Path
import numpy as np
from PIL import Image
import national_independent_mesh as national

ROOT=national.ROOT

def build(output):
 width=8000;n=4097;spacing=32000/(n-1)
 cx,cy=national.project(138.7274,35.3606);tx=int(cx//width)-1;ty=int(cy//width)-1
 xx,yy=np.meshgrid(tx*width+np.arange(-1,n+1)*spacing,(ty+4)*width-np.arange(-1,n+1)*spacing)
 binary=ROOT/'build/terrain-rust-target/release/terrain-adaptive-tin.exe'
 national.initialize(ROOT/'build/terrain-lod.sqlite3',str(binary))
 print('元 DEM から 32 km 四方を取得',flush=True)
 halo,valid=national.source_samples(xx,yy);grid=np.rint(halo[1:-1,1:-1]*4)/4;valid=valid[1:-1,1:-1]
 with tempfile.TemporaryDirectory(dir=ROOT/'build',prefix='joint-tin-') as temp:
  png=Path(temp)/'source.png';report=Path(temp)/'mesh.json';offset=min(0,float(grid.min()))
  Image.fromarray(np.rint((grid-offset)*4).astype(np.uint16)).save(png)
  print('区画内部に境界制約を置かず Greedy Insertion を実行',flush=True)
  subprocess.run([str(binary),'--input',str(png),'--output',str(report),'--scale','.25','--offset',str(offset),'--max-error','25','--min-vertices','2048','--max-vertices','32768'],check=True)
  mesh=json.loads(report.read_text(encoding='utf-8'))
 points=np.asarray(mesh['vertices'],dtype=float);faces=np.asarray(mesh['triangles'],dtype=np.uint32)
 # 同じ三角形分割から誤差を算出する。各区画を別々に再分割しない。
 from tin_vertex_optimize import raster
 owner,bary=raster(points,faces,n);prediction=(points[faces[owner],2]*bary).sum(axis=1)
 error=np.abs(prediction-grid.ravel());max_error=float(error[valid.ravel()].max())
 print('最大標高誤差',max_error,flush=True)
 dy,dx=np.gradient(halo,spacing)
 gx=np.rint(points[:,0]).astype(int)+1;gy=np.rint(points[:,1]).astype(int)+1
 slopes=np.column_stack([dx[gy,gx],dy[gy,gx]])
 world=points.copy();world[:,0]=tx*width+points[:,0]*spacing;world[:,1]=(ty+4)*width-points[:,1]*spacing
 a=world[faces[:,1],:2]-world[faces[:,0],:2];b=world[faces[:,2],:2]-world[faces[:,0],:2];cross=a[:,0]*b[:,1]-a[:,1]*b[:,0]
 faces[cross<0]=faces[cross<0][:,[0,2,1]]
 centers=world[faces,:2].mean(axis=1);ownership=np.floor(centers/width).astype(int)
 with sqlite3.connect(ROOT/'build/terrain-independent-national.sqlite3') as src,sqlite3.connect(output) as db:
  db.executescript('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE IF NOT EXISTS tiles(level INTEGER,x INTEGER,y INTEGER,geometry BLOB,hints BLOB,mask BLOB,stats TEXT,PRIMARY KEY(level,x,y));DELETE FROM tiles;')
  meta=json.loads(src.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0]);meta['jointMesh']=dict(region='富士山32km',level=3,maxErrorMeters=max_error,vertices=len(points),triangles=len(faces),vertexLimitPerPatch=32768,sampleSpacingMeters=spacing,bounds=[tx*width,(tx+4)*width,ty*width,(ty+4)*width])
  db.execute('INSERT OR REPLACE INTO metadata VALUES(?,?)',('manifest',json.dumps(meta)))
  total=0;crossing=0;counts=[]
  for x in range(tx,tx+4):
   for y in range(ty,ty+4):
    chosen=faces[(ownership[:,0]==x)&(ownership[:,1]==y)];ids,local=np.unique(chosen,return_inverse=True);local=local.reshape(-1,3).astype('<u2');p=world[ids].copy();p[:,:2]-=[x*width,y*width]
    # YNT2 は区画外まで伸びる頂点を、区画基準のメートル座標で保持する。
    geometry=zlib.compress(struct.pack('<4sHHII',b'YNT2',1025,1025,len(ids),local.size)+p.astype('<f4').tobytes()+local.tobytes(),9)
    h=np.rint(np.clip(slopes[ids],-8,8)/8*32767).astype('<i2');hints=zlib.compress(struct.pack('<4sIf',b'YNH1',len(ids),8.)+h.tobytes(),9)
    row=src.execute('SELECT mask FROM tiles WHERE level=3 AND x=? AND y=?',(x,y)).fetchone()
    if not row:raise ValueError('元の有効域マスクがありません')
    counts.append(len(ids));crossing+=int(np.any((p[:,:2]<0)|(p[:,:2]>width),axis=1).sum());total+=len(geometry)
    db.execute('INSERT INTO tiles VALUES(?,?,?,?,?,?,?)',(3,x,y,geometry,hints,row[0],json.dumps(dict(vertices=len(ids),triangles=len(chosen),joint=True))))
  meta['jointMesh']['minVerticesPerTile']=min(counts);meta['jointMesh']['maxVerticesPerTile']=max(counts)
  db.execute('INSERT OR REPLACE INTO metadata VALUES(?,?)',('manifest',json.dumps(meta)))
  db.commit()
 print(json.dumps(dict(output=str(output),tiles=16,geometryBytes=total,crossBoundaryVertexReferences=crossing,**meta['jointMesh']),ensure_ascii=False),flush=True)

if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('--output',type=Path,default=ROOT/'build/terrain-fuji-joint-l3.sqlite3');build(p.parse_args().output)
