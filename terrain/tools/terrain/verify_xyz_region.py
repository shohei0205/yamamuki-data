"""地域SQLiteをRustと独立した復元器で読み、全タイルの形式・接続を検査する。"""
import argparse,json,sqlite3,struct,time
from pathlib import Path

def varint(data,at,limit=5):
 value=0
 for i in range(limit):
  if at>=len(data):raise ValueError('可変長整数の切断')
  b=data[at];at+=1;value|=(b&127)<<(7*i)
  if b<128:return value,at
 raise ValueError('可変長整数超過')
def unzig(v):return -(v//2)-1 if v&1 else v//2

def references(mode,data,count,parents):
 if mode==0:
  if len(data)!=count*4:raise ValueError('生参照長')
  result=list(struct.unpack('<'+'I'*count,data))
 elif mode==1:
  if len(data)<2:raise ValueError('参照ヘッダー')
  groups=struct.unpack_from('<H',data)[0];at=2;result=[];previous=-1
  if not 0<groups<=len(parents):raise ValueError('グループ数')
  for _ in range(groups):
   slot=data[at];at+=1
   if not previous<slot<len(parents):raise ValueError('参照slot')
   previous=slot;n,at=varint(data,at);index,at=varint(data,at)
   if not 0<n<=count-len(result):raise ValueError('参照個数')
   for j in range(n):
    if j:delta,at=varint(data,at);index+=delta+1
    if index>=1<<24:raise ValueError('24bit参照超過')
    result.append(slot<<24|index)
  if at!=len(data) or len(result)!=count:raise ValueError('参照末尾・個数')
 else:raise ValueError('参照方式')
 if any(a>=b for a,b in zip(result,result[1:])):raise ValueError('参照順・重複')
 for v in result:
  slot,index=v>>24,v&0xffffff
  if slot>=len(parents) or index>=len(parents[slot]):raise ValueError('参照先範囲')
 return result

def tfc(data,count,nv):
 if not data or data[0]!=0xc2 or count>len(data)-1:raise ValueError('TFC2ヘッダー・個数')
 edges=[];recent=[];last=0;at=1;faces=[]
 for _ in range(count):
  code=data[at];at+=1
  if code==240:
   face=[];previous=last
   for _ in range(3):v,at=varint(data,at);previous+=unzig(v);face.append(previous)
   a,b,c=face
  elif code<240:
   edge,token=code>>4,code&15
   if edge>=len(edges):raise ValueError('共有辺範囲')
   a,b=edges[edge]
   if token<12:
    if token>=len(recent):raise ValueError('LRU範囲')
    c=recent[token]
   elif token==12:c=a+1
   elif token==13:c=b+1
   elif token==14:c=last+1
   else:v,at=varint(data,at);c=(b if v&1 else last)+unzig(v>>1)
  else:raise ValueError('TFC2制御値')
  if len({a,b,c})!=3 or min(a,b,c)<0 or max(a,b,c)>=nv:raise ValueError('面番号')
  faces.append((a,b,c));edges=([(b,a),(c,b),(a,c)]+edges)[:15]
  for v in (a,b,c):
   if v in recent:recent.remove(v)
   recent.insert(0,v);del recent[12:]
  last=c
 if at!=len(data):raise ValueError('TFC2末尾')
 return faces

def verify(path):
 started=time.monotonic();db=sqlite3.connect(Path(path).resolve().as_uri()+'?mode=ro',uri=True)
 if db.execute('pragma integrity_check').fetchone()[0]!='ok':raise ValueError('SQLite integrity')
 meta={k:json.loads(v) for k,v in db.execute('select key,value from metadata')}
 if meta['format']!='YMTM' or meta['format_version']!=4 or not meta['complete']:raise ValueError('DB形式・完了状態')
 cache={};depths={};maximum_depth=0
 def vertices(key,stack=()):
  nonlocal maximum_depth
  if key in stack or len(stack)>2:raise ValueError('参照循環・段数')
  maximum_depth=max(maximum_depth,len(stack))
  if key in cache:return cache[key]
  row=db.execute('select vertex_count,vertices from tiles where z=? and tile_x=? and tile_y=?',key).fetchone()
  if row is None:raise ValueError('親タイル欠落')
  nv,data=row
  if len(data)<32 or data[:4]!=b'YMV4':raise ValueError('頂点magic')
  ver,mode,v,d,r,a,p,res=struct.unpack_from('<HH6I',data,4)
  if ver!=4 or d>256 or v!=nv or v!=r+a or res or len(data)!=32+12*d+p+6*a:raise ValueError('頂点ヘッダー')
  if key[0] in (6,7,8,9,12) and (d or r or p or mode):raise ValueError('全頂点保存レベルの依存')
  deps=[struct.unpack_from('<Iii',data,32+12*i) for i in range(d)]
  if deps!=sorted(set(deps),key=lambda k:(k[0],k[2],k[1])) or any(k[0]!=key[0]-1 for k in deps):raise ValueError('依存表')
  parents=[vertices(k,stack+(key,)) for k in deps]
  at=32+12*d;refs=references(mode,data[at:at+p],r,parents)
  result=[parents[v>>24][v&0xffffff] for v in refs];k=1<<(14-key[0]);at+=p
  for i in range(a):x,y,h=struct.unpack_from('<hhH',data,at+6*i);result.append(((key[1]*16384+x)*k,(key[2]*16384+y)*k,h))
  if len({p[:2] for p in result})!=len(result):raise ValueError('頂点XY重複')
  cache[key]=result;depths[key]=0 if not deps else 1+max(depths[k] for k in deps)
  maximum_depth=max(maximum_depth,depths[key])
  if depths[key]>2:raise ValueError('親参照深度')
  return result
 rows=[]
 for z in range(6,15):
  if z in (6,7,8,9,12):cache.clear();depths.clear()
  seen={};seen_faces=set();edges={};tiles=faces_total=0;vb=tb=0;max_vertices=0
  for x,y,nv,nt,body,data in db.execute('select tile_x,tile_y,vertex_count,triangle_count,body_triangle_count,triangles from tiles where z=? order by tile_y,tile_x',(z,)):
   key=(z,x,y)
   if not (0<=x<1<<z and 0<=y<1<<z):raise ValueError('XYZ番号')
   points=vertices(key)
   if len(data)<24 or data[:4]!=b'YMT3':raise ValueError('面magic')
   ver,flags,t,b,l,h=struct.unpack_from('<HH4I',data,4)
   if ver!=3 or flags or t!=nt or b!=body or b>t or len(data)!=24+l+h:raise ValueError('面ヘッダー')
   faces=tfc(data[24:24+l],b,nv)+tfc(data[24+l:],t-b,nv)
   for point in points:
    xy=point[:2]
    if xy in seen and seen[xy]!=point[2]:raise ValueError('共有頂点の高さ不一致')
    seen[xy]=point[2]
   for face in faces:
    a,c,d=[points[v] for v in face]
    signed=(c[0]-a[0])*(d[1]-a[1])-(c[1]-a[1])*(d[0]-a[0])
    if signed>=0:raise ValueError('面の反転・退化')
    coordinates=tuple(points[v] for v in face);i=min(range(3),key=lambda i:coordinates[i]);canonical=coordinates[i:]+coordinates[:i]
    if canonical not in seen_faces:
     seen_faces.add(canonical)
     for u,v in zip(coordinates,coordinates[1:]+coordinates[:1]):
      edge=tuple(sorted((u,v)));edges[edge]=edges.get(edge,0)+1
      if edges[edge]>2:raise ValueError('同レベルの非多様体接続')
   tiles+=1;faces_total+=len(faces);max_vertices=max(max_vertices,nv);tb+=len(data)
  vb=db.execute('select coalesce(sum(length(vertices)),0) from tiles where z=?',(z,)).fetchone()[0]
  rows.append(dict(z=z,tiles=tiles,storedTriangles=faces_total,uniqueTriangles=len(seen_faces),uniqueVertices=len(seen),maximumTileVertices=max_vertices,vertexBytes=vb,triangleBytes=tb))
  print(json.dumps(rows[-1]),flush=True)
 packing=json.loads(Path(path).with_name('packing-report.json').read_text())
 result=dict(passed=True,quantizedGeometryPassed=all(r['quantizedAudit']['passed'] for r in packing),maximumParentDepth=maximum_depth,levels=rows,sqliteBytes=Path(path).stat().st_size,seconds=time.monotonic()-started,limitations=meta['test_limitations'])
 Path(path).with_name('independent-verification.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
 return result
if __name__=='__main__':
 parser=argparse.ArgumentParser();parser.add_argument('input');args=parser.parse_args();result=verify(args.input)
 print(json.dumps({k:v for k,v in result.items() if k!='levels'},ensure_ascii=False),flush=True)
 raise SystemExit(0 if result['quantizedGeometryPassed'] else 2)
