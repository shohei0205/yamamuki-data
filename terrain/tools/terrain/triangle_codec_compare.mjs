// 頂点番号を変えず、面の巡回順と共有辺を利用した圧縮を比較する。
import fs from 'node:fs';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
import {deflateSync,inflateSync} from 'node:zlib';
import {performance} from 'node:perf_hooks';
import assert from 'node:assert/strict';
import {encodeTriangles,decodeTriangles} from './triangle_frontier_codec.mjs';
import {createTfcDecoder} from './triangle_frontier_wasm.mjs';

const root=path.resolve(import.meta.dirname,'../..');
const out=path.join(root,'build/hierarchical-lod-20261023');
const vendor=path.join(root,'build/terrain-codec-tools/web/meshoptimizer');
const {MeshoptEncoder:E}=await import(pathToFileURL(path.join(vendor,'meshopt_encoder.js')));
const {MeshoptDecoder:D}=await import(pathToFileURL(path.join(vendor,'meshopt_decoder.mjs')));
await Promise.all([E.ready,D.ready]);
const wasmDecode=await createTfcDecoder(fs.readFileSync(path.join(out,'triangle_frontier_decoder.wasm')));

function ordered(indices){
 const count=indices.length/3, edges=new Map(),used=new Uint8Array(count),result=[];
 for(let i=0;i<count;i++)for(let j=0;j<3;j++){const a=indices[i*3+j],b=indices[i*3+(j+1)%3];edges.set(a*65536+b,i);}
 for(let first=0;first<count;first++){
  if(used[first])continue;
  const queue=[first];used[first]=1;
  for(let head=0;head<queue.length;head++){
   const i=queue[head];result.push(...indices.subarray(i*3,i*3+3));
   for(let j=0;j<3;j++){const a=indices[i*3+j],b=indices[i*3+(j+1)%3],n=edges.get(b*65536+a);if(n!==undefined&&!used[n]){used[n]=1;queue.push(n);}}
  }
 }
 return new Uint16Array(result);
}
function cacheOrdered(indices){
 const data=new Uint32Array(indices),[remap,count]=E.reorderMesh(data,true,true),inverse=new Uint32Array(count);
 for(let old=0;old<remap.length;old++)if(remap[old]<count)inverse[remap[old]]=old;
 return new Uint16Array(data.map(i=>inverse[i]));
}
function pushEdges(fifo,a,b,c){fifo.unshift([b,a],[c,b],[a,c]);fifo.length=Math.min(32,fifo.length);}
function edgeEncode(src){
 const bytes=[],fifo=[];let last=0;
 const integer=n=>{let v=n>=0?n*2:-n*2-1;while(v>=128){bytes.push((v&127)|128);v>>>=7;}bytes.push(v);};
 for(let i=0;i<src.length;i+=3){let a=src[i],b=src[i+1],c=src[i+2],found=-1;
  for(let k=0;k<fifo.length;k++){const[x,y]=fifo[k];if(x===a&&y===b){found=k;break;}if(x===b&&y===c){[a,b,c]=[b,c,a];found=k;break;}if(x===c&&y===a){[a,b,c]=[c,a,b];found=k;break;}}
  if(found<0){bytes.push(32);integer(a-last);integer(b-a);integer(c-b);}else{bytes.push(found);integer(c-last);}
  last=c;pushEdges(fifo,a,b,c);
 }
 return new Uint8Array(bytes);
}
function edgeDecode(src,count){
 const out=new Uint16Array(count),fifo=[];let cursor=0,last=0;
 const integer=()=>{let v=0,shift=0,b;do{assert(cursor<src.length);b=src[cursor++];v|=(b&127)<<shift;shift+=7;assert(shift<=28);}while(b&128);return v&1?-(v+1)/2:v/2;};
 for(let i=0;i<count;i+=3){let a,b,c;const code=src[cursor++];if(code===32){a=last+integer();b=a+integer();c=b+integer();}else{assert(code<fifo.length);[a,b]=fifo[code];c=last+integer();}assert(a>=0&&a<65536&&b>=0&&b<65536&&c>=0&&c<65536);out.set([a,b,c],i);last=c;pushEdges(fifo,a,b,c);}
 assert.equal(cursor,src.length);return out;
}
function canonical(indices){const out=[];for(let i=0;i<indices.length;i+=3){const f=[...indices.subarray(i,i+3)],min=Math.min(...f),k=f.indexOf(min);out.push([f[k],f[(k+1)%3],f[(k+2)%3]].join(','));}return out.sort().join(';');}
const bytes=a=>new Uint8Array(a.buffer,a.byteOffset,a.byteLength);
const codecs={
 zlib:{name:'現行：uint16＋zlib',encode:a=>deflateSync(bytes(a),{level:6}),decode:(b,n)=>new Uint16Array(Uint8Array.from(inflateSync(b)).buffer)},
 orderedZlib:{name:'隣接面順＋zlib',order:true,encode:a=>deflateSync(bytes(a),{level:6}),decode:(b,n)=>new Uint16Array(Uint8Array.from(inflateSync(b)).buffer)},
 edgeZlib:{name:'共有辺FIFO＋zlib',order:true,encode:a=>deflateSync(edgeEncode(a),{level:6}),decode:(b,n)=>edgeDecode(inflateSync(b),n)},
 meshopt:{name:'meshoptimizer',encode:a=>E.encodeIndexBuffer(bytes(a),a.length,2),decode:(b,n)=>{const t=new Uint8Array(n*2);D.decodeIndexBuffer(t,n,2,b);return new Uint16Array(t.buffer);}},
 meshoptZlib:{name:'meshoptimizer＋zlib',encode:a=>deflateSync(E.encodeIndexBuffer(bytes(a),a.length,2),{level:6}),decode:(b,n)=>{const t=new Uint8Array(n*2);D.decodeIndexBuffer(t,n,2,inflateSync(b));return new Uint16Array(t.buffer);}},
 orderedMeshopt:{name:'隣接面順＋meshoptimizer',order:true,encode:a=>E.encodeIndexBuffer(bytes(a),a.length,2),decode:(b,n)=>{const t=new Uint8Array(n*2);D.decodeIndexBuffer(t,n,2,b);return new Uint16Array(t.buffer);}},
 orderedMeshoptZlib:{name:'隣接面順＋meshoptimizer＋zlib',order:true,encode:a=>deflateSync(E.encodeIndexBuffer(bytes(a),a.length,2),{level:6}),decode:(b,n)=>{const t=new Uint8Array(n*2);D.decodeIndexBuffer(t,n,2,inflateSync(b));return new Uint16Array(t.buffer);}}
};
codecs.cacheZlib={...codecs.zlib,name:'公式の面順最適化＋zlib',order:'cache'};
codecs.cacheMeshopt={...codecs.meshopt,name:'公式の面順最適化＋meshoptimizer',order:'cache'};
codecs.cacheMeshoptZlib={...codecs.meshoptZlib,name:'公式の面順最適化＋meshoptimizer＋zlib',order:'cache'};
codecs.frontierOriginal={name:'独自TFC1・元の面順（zlibなし）',encode:a=>encodeTriangles(a,false),decode:decodeTriangles};
codecs.frontierGreedy={name:'独自TFC1・共有辺から面順も選択（zlibなし）',encode:a=>encodeTriangles(a,true),decode:decodeTriangles};
codecs.frontierCache={name:'公式の面順最適化＋独自TFC1（zlibなし）',order:'cache',encode:a=>encodeTriangles(a,false),decode:decodeTriangles};
codecs.frontierWasm={...codecs.frontierGreedy,name:'独自TFC1・独自面順＋WASM（zlibなし）',decode:wasmDecode};
codecs.frontierCacheWasm={...codecs.frontierCache,name:'公式の面順最適化＋独自TFC1 WASM（zlibなし）',decode:wasmDecode};
// 小さな合成面でも向き・番号の保持を検証する。
for(const test of [new Uint16Array([0,1,2]),new Uint16Array([0,1,2,2,1,3,4,5,6]),new Uint16Array([65533,65534,65535])])for(const codec of Object.values(codecs)){const data=codec.order==='cache'?cacheOrdered(test):codec.order?ordered(test):test;assert.equal(canonical(codec.decode(codec.encode(data),data.length)),canonical(test));}
const input=JSON.parse(fs.readFileSync(path.join(out,'triangle-codec-input.json'))),records=[];
for(const row of input){const raw=Uint8Array.from(Buffer.from(row.indices,'base64')),src=new Uint16Array(raw.buffer),sorted=ordered(src),cache=cacheOrdered(src),expected=canonical(src),encoded={};
 for(const[key,c]of Object.entries(codecs)){const blob=c.encode(c.order==='cache'?cache:c.order?sorted:src);assert.equal(canonical(c.decode(blob,src.length)),expected,`${row.region}/${row.level}/${row.tile}/${key}`);encoded[key]=Buffer.from(blob).toString('base64');}
 records.push({...row,indices:undefined,indexCount:src.length,encoded});
}
const groups=[];
for(const region of ['all','fuji','yari','tsukuba'])for(const level of ['all','0','1','2','3','mixed']){
 const rows=records.filter(r=>(region==='all'||r.region===region)&&(level==='all'||r.level===level));if(!rows.length)continue;
 const summary={region,level,tiles:rows.length,triangles:rows.reduce((s,r)=>s+r.indexCount/3,0),vertexBytes:rows.reduce((s,r)=>s+r.vertexBytes,0),codecs:{}};
 for(const[key,c]of Object.entries(codecs)){
  const blobs=rows.map(r=>({n:r.indexCount,b:Buffer.from(r.encoded[key],'base64')}));
  for(let j=0;j<5;j++)for(const x of blobs)c.decode(x.b,x.n);
  const samples=[];let checksum=0;
  for(let j=0;j<15;j++){const start=performance.now();for(const x of blobs){const a=c.decode(x.b,x.n);checksum+=a[0];}samples.push(performance.now()-start);}
  samples.sort((a,b)=>a-b);summary.codecs[key]={name:c.name,bytes:blobs.reduce((s,x)=>s+x.b.length,0),decodeMs:samples[7],checksum};
 }
 groups.push(summary);
}
fs.writeFileSync(path.join(out,'triangle-codec-blobs.json'),JSON.stringify(records));
fs.writeFileSync(path.join(out,'triangle-codec-report.json'),JSON.stringify({meshoptimizerVersion:JSON.parse(fs.readFileSync(path.join(vendor,'package.json'))).version,codecCount:Object.keys(codecs).length,verifiedTiles:records.length,groups},null,2)+'\n');
console.log(JSON.stringify(groups.find(g=>g.region==='all'&&g.level==='all'),null,2));
