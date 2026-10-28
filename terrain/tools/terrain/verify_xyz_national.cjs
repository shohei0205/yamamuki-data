'use strict';
// Rustの生成器を使わず、ブラウザー用復元器と行単位の接続索引で全国を検証する。
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
const {DatabaseSync}=require('node:sqlite');
const {pathToFileURL}=require('node:url');
const {decodeYMV}=require('./xyz_viewer.js');
const FULL=new Set([6,7,8,9,12]),GRID=4194304,ID_BASE=2**26;
function ensure(ok,message){if(!ok)throw Error(message)}
function json(file,value){fs.writeFileSync(file,JSON.stringify(value,null,2)+'\n')}
function intersects(a,b,c,box){
 if(Math.max(a.x,b.x,c.x)<box[0]||Math.min(a.x,b.x,c.x)>box[2]||Math.max(a.y,b.y,c.y)<box[1]||Math.min(a.y,b.y,c.y)>box[3])return false;
 for(const [u,v] of [[a,b],[b,c],[c,a]]){const dx=v.x-u.x,dy=v.y-u.y;const min=dx*((dx>=0?box[1]:box[3])-u.y)-dy*((dy>=0?box[2]:box[0])-u.x);if(min>0)return false}return true;
}
function validateRefs(vb,parents,r,d,p,mode){
 const at=32+12*d,end=at+p;let refs=[];
 if(mode===0){ensure(p===r*4,'生参照長');for(let i=0;i<r;i++)refs.push(vb.readUInt32LE(at+i*4))}
 else if(mode===1){ensure(p>=2,'参照ヘッダー');let cursor=at+2;const groups=vb.readUInt16LE(at);ensure(groups>0&&groups<=d,'参照グループ数');let lastSlot=-1;
  function variable(){let value=0,mul=1;for(let i=0;i<5;i++){ensure(cursor<end,'参照整数の切断');const b=vb[cursor++];value+=(b&127)*mul;if(b<128)return value;mul*=128}throw Error('参照整数超過')}
  for(let i=0;i<groups;i++){ensure(cursor<end,'参照slotの切断');const slot=vb[cursor++];ensure(slot>lastSlot&&slot<d,'参照slot順');lastSlot=slot;const n=variable();let index=variable();ensure(n>0&&refs.length+n<=r,'参照個数');for(let j=0;j<n;j++){if(j)index+=variable()+1;ensure(index<2**24,'参照index超過');refs.push(slot*2**24+index)}}ensure(cursor===end,'参照末尾');
 }else throw Error('参照方式');
 ensure(refs.length===r,'参照個数不一致');for(let i=0;i<refs.length;i++){ensure(i===0||refs[i]>refs[i-1],'参照順・重複');const slot=Math.floor(refs[i]/2**24),index=refs[i]%2**24;ensure(slot<d&&index<parents[slot].points.length/3,'親参照範囲')}
}
async function verify(input,options={}){
 const directory=path.dirname(path.resolve(input)),config=JSON.parse(fs.readFileSync(path.join(directory,'national-config.json'))),chunks=new Set(config.chunks.map(p=>p.join(','))),chunkSpan=config.cells*64;
 const reportFile=options.report||path.join(directory,'independent-national-verification.json'),progressFile=options.progress||path.join(directory,'verification.progress.json');
 const db=new DatabaseSync(path.resolve(input),{readOnly:true});db.exec('PRAGMA cache_size=-65536');
 const started=performance.now(),output={passed:false,input:path.resolve(input),levels:[],limitations:['元標高との誤差は生成時の全標本検証結果を別途参照する。','混在LOD接合・Loop細分割は、この同レベル接続検査の対象外。']};
 try{
  json(progressFile,{stage:'integrity-check',complete:false});ensure(db.prepare('PRAGMA integrity_check').get().integrity_check==='ok','SQLite整合性');
  ensure(db.prepare('PRAGMA user_version').get().user_version===4,'SQLite版');ensure(db.prepare('PRAGMA application_id').get().application_id===1498240077,'SQLite識別子');
  const meta=Object.fromEntries(db.prepare('SELECT key,value FROM metadata').all().map(r=>[r.key,JSON.parse(r.value)]));ensure(meta.format==='YMTM'&&meta.format_version===4&&meta.complete,'形式・完了状態');
  const {createXYZDecoder}=await import(pathToFileURL(path.join(__dirname,'xyz_wasm.mjs')).href),wasm=await createXYZDecoder(fs.readFileSync(path.resolve(__dirname,'../../build/xyz_decoder.wasm')));
  const getVertex=db.prepare('SELECT vertex_count,vertices FROM tiles WHERE z=? AND tile_x=? AND tile_y=?');let cache=new Map(),cacheBytes=0,maxDepth=0;
  async function resolve(z,x,y,stack=[]){
   const key=`${z}/${x}/${y}`;ensure(!stack.includes(key)&&stack.length<=2,'親参照循環・段数');if(cache.has(key)){const record=cache.get(key);cache.delete(key);cache.set(key,record);return record}
   const row=getVertex.get(z,x,y);ensure(row,'親タイル欠落 '+key);const vb=Buffer.from(row.vertices);ensure(vb.length>=32&&vb.toString('ascii',0,4)==='YMV4'&&vb.readUInt16LE(4)===4,'頂点形式 '+key);
   const mode=vb.readUInt16LE(6),nv=vb.readUInt32LE(8),d=vb.readUInt32LE(12),r=vb.readUInt32LE(16),a=vb.readUInt32LE(20),p=vb.readUInt32LE(24);
   ensure(d<=256&&nv===row.vertex_count&&nv===r+a&&nv<=2**24&&vb.readUInt32LE(28)===0&&vb.length===32+12*d+p+6*a,'頂点ヘッダー '+key);
   if(FULL.has(z))ensure(d===0&&r===0&&p===0&&mode===0,'独立レベルの依存 '+key);
   const parents=[];let previous=null,depth=0;
   for(let i=0;i<d;i++){const at=32+12*i,dep=[vb.readUInt32LE(at),vb.readInt32LE(at+4),vb.readInt32LE(at+8)];ensure(dep[0]===z-1&&dep[1]>=0&&dep[1]<2**dep[0]&&dep[2]>=0&&dep[2]<2**dep[0],'親レベル・番号 '+key);ensure(!previous||dep[2]>previous[2]||(dep[2]===previous[2]&&dep[1]>previous[1]),'親依存表の順序・重複 '+key);previous=dep;const parent=await resolve(...dep,[...stack,key]);parents.push(parent);depth=Math.max(depth,parent.depth+1)}
   ensure(depth<=({10:1,11:2,13:1,14:2}[z]||0),'親深度 '+key);maxDepth=Math.max(maxDepth,depth);validateRefs(vb,parents,r,d,p,mode);
   const points=await decodeYMV(vb,z,x,y,(zz,xx,yy)=>resolve(zz,xx,yy,[...stack,key]));const record={points,depth};cache.set(key,record);cacheBytes+=points.byteLength;
   while(cacheBytes>128*1048576&&cache.size>1){const oldest=cache.keys().next().value;cacheBytes-=cache.get(oldest).points.byteLength;cache.delete(oldest)}return record;
  }
  const levels=options.levels||[6,7,8,9,10,11,12,13,14];
  for(const z of levels){
   cache.clear();cacheBytes=0;const span=16384*2**(14-z),valid=new Set(db.prepare('SELECT tile_x,tile_y FROM tiles WHERE z=?').all(z).map(r=>r.tile_x*16384+r.tile_y)),total=valid.size;
   const points=new Map(),faces=new Set(),edges=new Map(),pointBuckets=new Map(),faceBuckets=new Map(),edgeBuckets=new Map();let nextID=0,uniqueFaces=0,openEdges=0,checked=0,storedFaces=0,vertexBytes=0,triangleBytes=0,lastRow=-Infinity,lastProgress=0,peakPoints=0,peakEdges=0,maxCos=-1;
   function bucket(map,row,key){let list=map.get(row);if(!list)map.set(row,list=[]);list.push(key)}
   function externalChunk(a,b){if(a.x===b.x&&a.x%chunkSpan===0){const x=a.x/chunkSpan,y=Math.floor((a.y+b.y)/2/chunkSpan);if(!chunks.has(`${x-1},${y}`)||!chunks.has(`${x},${y}`))return true}if(a.y===b.y&&a.y%chunkSpan===0){const y=a.y/chunkSpan,x=Math.floor((a.x+b.x)/2/chunkSpan);if(!chunks.has(`${x},${y-1}`)||!chunks.has(`${x},${y}`))return true}return false}
   function finishEdge(edge){const [a,b,count]=edge;if(count===2)return;ensure(count===1,'非多様体辺');openEdges++;if(externalChunk(a,b))return;const mx=(a.x+b.x)/2,my=(a.y+b.y)/2,dx=b.x-a.x,dy=b.y-a.y,scale=.25/Math.max(Math.abs(dx),Math.abs(dy)),x1=Math.floor((mx-dy*scale)/span),y1=Math.floor((my+dx*scale)/span),x2=Math.floor((mx+dy*scale)/span),y2=Math.floor((my-dx*scale)/span);ensure(!(valid.has(x1*16384+y1)&&valid.has(x2*16384+y2)),`内部の片面辺 z${z}: ${a.x},${a.y} / ${b.x},${b.y}`)}
   function flush(row){for(const [key,list] of edgeBuckets){if(key>=row-2)continue;for(const id of list){finishEdge(edges.get(id));edges.delete(id)}edgeBuckets.delete(key)}for(const [key,list] of faceBuckets){if(key>=row-2)continue;for(const id of list)faces.delete(id);faceBuckets.delete(key)}for(const [key,list] of pointBuckets){if(key>=row-2)continue;for(const id of list)points.delete(id);pointBuckets.delete(key)}}
   function edge(a,b){if(a.id>b.id)[a,b]=[b,a];const key=a.id*ID_BASE+b.id;let record=edges.get(key);if(record){record[2]++;ensure(record[2]<=2,'非多様体辺 z'+z)}else{edges.set(key,[a,b,1]);bucket(edgeBuckets,Math.floor(Math.min(a.y,b.y)/span),key)}}
   for(const row of db.prepare('SELECT tile_x,tile_y,mesh_id,vertex_count,triangle_count,body_triangle_count,vertices,triangles FROM tiles WHERE z=? ORDER BY tile_y,tile_x').iterate(z)){
    try{
     const x=row.tile_x,y=row.tile_y;ensure(x>=0&&x<2**z&&y>=0&&y<2**z,'XYZ番号');ensure(row.mesh_id===`${config.mesh_id}-z${z}`,'メッシュ識別子');if(y!==lastRow){flush(y);lastRow=y}
     const {points:pp}=await resolve(z,x,y),local=[],seen=new Set();ensure(pp.length===row.vertex_count*3,'頂点個数');
     for(let i=0;i<pp.length;i+=3){const px=pp[i],py=pp[i+1],h=pp[i+2];ensure(Number.isSafeInteger(px)&&Number.isSafeInteger(py)&&px>=0&&py>=0&&px<2**28&&py<2**28&&px%64===0&&py%64===0,'共通格子の座標');ensure(h>=0&&h<=65535&&Number.isInteger(h),'標高値');const lx=(px-x*span)/2**(14-z),ly=(py-y*span)/2**(14-z);ensure(lx>=-32768&&lx<32768&&ly>=-32768&&ly<32768,'余白の座標範囲');const key=px/64*GRID+py/64;ensure(!seen.has(key),'タイル内の重複頂点');seen.add(key);let record=points.get(key);if(record)ensure(record.h===h,'共有頂点の高さ不一致');else{ensure(nextID<ID_BASE,'頂点索引の検証上限');record={id:nextID++,x:px,y:py,h};points.set(key,record);bucket(pointBuckets,Math.floor(py/span),key)}local.push(record)}
     const tb=Buffer.from(row.triangles),nv=local.length;ensure(tb.length>=24&&tb.toString('ascii',0,4)==='YMT3'&&tb.readUInt16LE(4)===3&&tb.readUInt16LE(6)===0,'面形式');const nt=tb.readUInt32LE(8),body=tb.readUInt32LE(12),l=tb.readUInt32LE(16),h=tb.readUInt32LE(20);ensure(nt===row.triangle_count&&body===row.body_triangle_count&&body<=nt&&tb.length===24+l+h,'面ヘッダー');const bf=wasm.decodeTFC(tb.subarray(24,24+l),body,nv),hf=wasm.decodeTFC(tb.subarray(24+l),nt-body,nv),bodyEdges=new Set(),tileFaces=new Set(),box=[x*span,y*span,(x+1)*span,(y+1)*span];
     for(const [part,ff] of [[0,bf],[1,hf]])for(let i=0;i<ff.length;i+=3){const ia=ff[i],ib=ff[i+1],ic=ff[i+2],a=local[ia],b=local[ib],c=local[ic];const cross=(b.x-a.x)*(c.y-a.y)-(b.y-a.y)*(c.x-a.x);ensure(cross<0,'面の反転・退化');ensure(intersects(a,b,c,box)===(part===0),'本体・余白の分類');const ek=(u,v)=>Math.min(u,v)*nv+Math.max(u,v),ee=[ek(ia,ib),ek(ib,ic),ek(ic,ia)];if(part===0)for(const k of ee)bodyEdges.add(k);else ensure(ee.some(k=>bodyEdges.has(k)),'外側1層でない余白面');let ids=[a.id,b.id,c.id].sort((u,v)=>u-v),key=ids.join(',');ensure(!tileFaces.has(key),'タイル内の重複面');tileFaces.add(key);if(faces.has(key))continue;faces.add(key);bucket(faceBuckets,Math.floor(Math.min(a.y,b.y,c.y)/span),key);uniqueFaces++;edge(a,b);edge(b,c);edge(c,a);
      const u=[b.x-a.x,b.y-a.y],v=[c.x-b.x,c.y-b.y],w=[a.x-c.x,a.y-c.y],uu=u[0]**2+u[1]**2,vv=v[0]**2+v[1]**2,ww=w[0]**2+w[1]**2;maxCos=Math.max(maxCos,-(u[0]*w[0]+u[1]*w[1])/Math.sqrt(uu*ww),-(u[0]*v[0]+u[1]*v[1])/Math.sqrt(uu*vv),-(v[0]*w[0]+v[1]*w[1])/Math.sqrt(vv*ww));
     }
     checked++;storedFaces+=nt;vertexBytes+=row.vertices.length;triangleBytes+=tb.length;peakPoints=Math.max(peakPoints,points.size);peakEdges=Math.max(peakEdges,edges.size);
     if(performance.now()-lastProgress>1000||checked===total){json(progressFile,{stage:'verify-tiles',z,processed:checked,total,storedTriangles:storedFaces,seconds:(performance.now()-started)/1000,residentMiB:process.memoryUsage().rss/1048576,complete:false});lastProgress=performance.now()}
    }catch(error){throw Error(`z${z}/${row.tile_x}/${row.tile_y}: ${error.message}`,{cause:error})}
   }
   flush(Infinity);const minimumAngle=Math.acos(Math.min(1,maxCos))*180/Math.PI;ensure(minimumAngle>=10-1e-6,'最小角10度未満');const result={z,tiles:checked,storedTriangles:storedFaces,uniqueTriangles:uniqueFaces,uniqueVertices:nextID,openBoundaryEdges:openEdges,minimumAngleDegrees:minimumAngle,vertexBytes,triangleBytes,peakRetainedVertices:peakPoints,peakRetainedEdges:peakEdges};output.levels.push(result);json(reportFile,output);console.log(JSON.stringify(result));
  }
  output.passed=true;output.maximumParentDepth=maxDepth;output.sqliteBytes=fs.statSync(input).size;output.seconds=(performance.now()-started)/1000;json(reportFile,output);json(progressFile,{stage:'verified',complete:true,...output});return output;
 }catch(error){output.error=error.stack;json(reportFile,output);json(progressFile,{stage:'failed',complete:false,error:error.stack});throw error}finally{db.close()}
}
module.exports={verify,intersects};
if(require.main===module)verify(process.argv[2]).then(r=>console.log(JSON.stringify({passed:r.passed,seconds:r.seconds}))).catch(e=>{console.error(e);process.exitCode=1});
