"""32 kmタイルを比高優先度と7段階の頂点上限で比較する。"""
import json,sqlite3,math,argparse,struct,zlib
import numpy as np
from concurrent.futures import ProcessPoolExecutor,as_completed
import compare_priority_from_start as compare
ROOT=compare.ROOT
BUDGETS=[1000,2000,4000,8000,16000,32000,64000]
def centered_tile(row,budget,offset):
 original=compare.national.source_samples
 compare.national.source_samples=lambda x,y:original(x+offset[0],y+offset[1])
 try:
  l,x,y,g,h,m,stats=row;n=2049;width=32000.;axis=np.linspace(0,width,n);xx,yy=np.meshgrid(x*width+axis,y*width+axis);grid,valid=compare.national.source_samples(xx,yy)
  raw=zlib.decompress(g);nv=struct.unpack_from('<I',raw,8)[0];old=np.frombuffer(raw,dtype='<f4',count=nv*3,offset=16).reshape(-1,3);points=old[np.any((old[:,:2]==0)|(old[:,:2]==width),axis=1)].copy();ij=np.rint(points[:,:2]*(n-1)/width).astype(int);points[:,2]=grid[ij[:,1],ij[:,0]];faces=compare.quality.triangulate(points).astype('<u2')
  geom=zlib.compress(struct.pack('<4sHHII',b'YNT2',n,n,len(points),faces.size)+points.astype('<f4').tobytes()+faces.tobytes(),9);mask=zlib.compress(np.packbits(valid,bitorder='little').tobytes(),9)
  return compare.generate_tile((l,x,y,geom,h,mask,stats),budget)
 finally:compare.national.source_samples=original

def main(centered=False):
 src=sqlite3.connect(ROOT/'build/terrain-regional-priority-levels-v5.sqlite3');manifest=json.loads(src.execute('SELECT value FROM metadata WHERE key="manifest"').fetchone()[0]);rows=src.execute('SELECT * FROM tiles WHERE level=5').fetchall();offsets={}
 for area in manifest['reliefRegions']:
  spec=next(v for v in area['levels'] if v['level']==5);l,x,y=spec['keys'][0];cx,cy=compare.national.project(area['lon'],area['lat']);offsets[x,y]=[float(cx-(x+.5)*32000),float(cy-(y+.5)*32000)];area['comparisonOffset']=offsets[x,y] if centered else [0,0]
 prefix='terrain-priority-centered-budget' if centered else 'terrain-priority-budget'
 with ProcessPoolExecutor(max_workers=2,initializer=compare.initialize) as pool:
  for budget in BUDGETS:
   db=sqlite3.connect(ROOT/f'build/{prefix}-{budget}.sqlite3');db.executescript('CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE IF NOT EXISTS tiles(level,x,y,geometry,hints,mask,stats,PRIMARY KEY(level,x,y));')
   pending=[r for r in rows if not (saved:=db.execute('SELECT stats FROM tiles WHERE level=? AND x=? AND y=?',r[:3]).fetchone()) or (centered and json.loads(saved[0]).get('centeredBoundaryVersion')!=2)]
   for future in as_completed([pool.submit(centered_tile,r,budget,offsets[tuple(r[1:3])]) if centered else pool.submit(compare.generate_tile,r,budget) for r in pending]):
    _,row,stats=future.result()[0]
    if centered:
     stats['centeredBoundaryVersion']=2;row=(*row[:6],json.dumps(stats))
    db.execute('INSERT OR REPLACE INTO tiles VALUES(?,?,?,?,?,?,?)',row);db.commit();print(budget,row[:3],stats['seconds'],flush=True)
   m=json.loads(json.dumps(manifest));m['levels']=[v for v in m['levels'] if v['level']==5];m.update(priorityComparison=True,priorityMode='unified',vertexBudget=budget,priorityComparisonPolicy='32km-centered-fixed-budget-v2' if centered else '32km-fixed-budget-v1',mountainCentered=centered)
   for area in m['reliefRegions']:
    area['levels']=[v for v in area['levels'] if v['level']==5]
    for spec in area['levels']:
     stats=[json.loads(db.execute('SELECT stats FROM tiles WHERE level=? AND x=? AND y=?',k).fetchone()[0]) for k in spec['keys']];spec['addedPositions']=[];spec['priorityMetrics']=dict(maximum=max(s['metrics']['maximum'] for s in stats),rmse=math.sqrt(sum(s['metrics']['rmse']**2 for s in stats)/len(stats)),weightedMaximum=max(s['metrics']['weightedMaximum'] for s in stats),vertices=sum(s['vertices'] for s in stats),geometryBytes=sum(s['geometryBytes'] for s in stats))
   db.execute('INSERT OR REPLACE INTO metadata VALUES("manifest",?)',(json.dumps(m),));db.commit();db.close();print('完了',budget,flush=True)
if __name__=='__main__':
 parser=argparse.ArgumentParser();parser.add_argument('--centered',action='store_true');main(parser.parse_args().centered)
