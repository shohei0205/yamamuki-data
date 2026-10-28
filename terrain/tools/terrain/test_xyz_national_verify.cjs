'use strict';
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),{DatabaseSync}=require('node:sqlite');
const {verify}=require('./verify_xyz_national.cjs'),{decodeYMV,decodeTFC}=require('./xyz_viewer.js');
(async()=>{
 const source=path.resolve(process.argv[2]),output=path.resolve(process.argv[3]);assert(!fs.existsSync(output));fs.mkdirSync(output,{recursive:true});
 const baseline=await verify(path.join(source,'terrain.sqlite3'),{report:path.join(output,'baseline.json'),progress:path.join(output,'baseline-progress.json')});
 const python=JSON.parse(fs.readFileSync(path.join(source,'independent-verification.json')));
 for(let i=0;i<9;i++)for(const key of ['tiles','storedTriangles','uniqueTriangles','uniqueVertices','vertexBytes','triangleBytes'])assert.equal(baseline.levels[i][key],python.levels[i][key],key);
 function copy(name){const dir=path.join(output,name);fs.mkdirSync(dir);fs.copyFileSync(path.join(source,'national-config.json'),path.join(dir,'national-config.json'));const db=new DatabaseSync(path.join(source,'terrain.sqlite3'),{readOnly:true});db.prepare('VACUUM INTO ?').run(path.join(dir,'terrain.sqlite3'));db.close();return dir}
 const heightDir=copy('bad-shared-height'),heightDB=new DatabaseSync(path.join(heightDir,'terrain.sqlite3'));
 const full=heightDB.prepare('SELECT tile_x,tile_y,vertices FROM tiles WHERE z=12 ORDER BY tile_y,tile_x').all();assert(full.length>1);let changed=false;
 for(let a=0;a<full.length&&!changed;a++)for(let b=a+1;b<full.length&&!changed;b++){
  const left=Buffer.from(full[a].vertices),right=Buffer.from(full[b].vertices),leftPoints=await decodeYMV(left,12,full[a].tile_x,full[a].tile_y,()=>{throw Error('独立レベル')}),rightPoints=await decodeYMV(right,12,full[b].tile_x,full[b].tile_y,()=>{throw Error('独立レベル')});const keys=new Set();for(let i=0;i<rightPoints.length;i+=3)keys.add(rightPoints[i]+','+rightPoints[i+1]);for(let i=0;i<leftPoints.length;i+=3)if(keys.has(leftPoints[i]+','+leftPoints[i+1])){const at=32+(i/3)*6+4;left.writeUInt16LE(left.readUInt16LE(at)+1,at);heightDB.prepare('UPDATE tiles SET vertices=? WHERE z=12 AND tile_x=? AND tile_y=?').run(left,full[a].tile_x,full[a].tile_y);changed=true;break}
 }
 heightDB.close();assert(changed);await assert.rejects(verify(path.join(heightDir,'terrain.sqlite3'),{levels:[12]}),/共有頂点の高さ不一致/);
 const holeDir=copy('bad-internal-hole'),db=new DatabaseSync(path.join(holeDir,'terrain.sqlite3')),cache=new Map();
 async function resolve(z,x,y){const key=[z,x,y].join(',');if(cache.has(key))return cache.get(key);const row=db.prepare('SELECT vertices FROM tiles WHERE z=? AND tile_x=? AND tile_y=?').get(z,x,y);const value={points:await decodeYMV(Buffer.from(row.vertices),z,x,y,resolve)};cache.set(key,value);return value}
 function literal(indices){const out=[194];let last=0;for(let i=0;i<indices.length;i+=3){out.push(240);let previous=last;for(let j=0;j<3;j++){const value=indices[i+j]-previous;let v=value<0?-value*2-1:value*2;while(v>=128){out.push((v%128)|128);v=Math.floor(v/128)}out.push(v);previous=indices[i+j]}last=indices[i+2]}return Buffer.from(out)}
 changed=false;
 for(const row of db.prepare('SELECT tile_x,tile_y,vertices,triangles FROM tiles WHERE z=14').all()){
  const pp=(await resolve(14,row.tile_x,row.tile_y)).points,tb=Buffer.from(row.triangles),body=tb.readUInt32LE(12),length=tb.readUInt32LE(16),indices=decodeTFC(tb.subarray(24,24+length),body,pp.length/3),xmin=row.tile_x*16384,ymin=row.tile_y*16384;
  for(let i=0;i<indices.length;i+=3){if(!Array.from(indices.subarray(i,i+3)).every(v=>pp[v*3]>xmin+16384*.2&&pp[v*3]<xmin+16384*.8&&pp[v*3+1]>ymin+16384*.2&&pp[v*3+1]<ymin+16384*.8))continue;
   const reduced=Array.from(indices);reduced.splice(i,3);const encoded=literal(reduced),header=Buffer.from(tb.subarray(0,24));header.writeUInt32LE(header.readUInt32LE(8)-1,8);header.writeUInt32LE(body-1,12);header.writeUInt32LE(encoded.length,16);const result=Buffer.concat([header,encoded,tb.subarray(24+length)]);db.prepare('UPDATE tiles SET triangles=?,triangle_count=triangle_count-1,body_triangle_count=body_triangle_count-1 WHERE z=14 AND tile_x=? AND tile_y=?').run(result,row.tile_x,row.tile_y);changed=true;break}
  if(changed)break;
 }
 db.close();assert(changed);await assert.rejects(verify(path.join(holeDir,'terrain.sqlite3'),{levels:[14]}),/内部の片面辺/);console.log('Pythonとの全レベル一致・共有高さ破損・内部欠損の検出に成功');
})().catch(e=>{console.error(e);process.exitCode=1});
