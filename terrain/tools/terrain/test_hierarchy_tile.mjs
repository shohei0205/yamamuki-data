// Rustの最終BLOBをブラウザー用復元器で読み、独立した作業メッシュと比較する。
import {readFile} from 'node:fs/promises';
import {inflateSync} from 'node:zlib';
import vm from 'node:vm';
import assert from 'node:assert/strict';
import {createTfcDecoder} from './triangle_frontier_wasm.mjs';
const fixture=JSON.parse(await readFile(process.argv[2]||'build/hierarchy-browser-fixture.json','utf8'));
const entries=new Map(fixture.map(v=>[v.key,v]));
const wasm=await readFile(new URL('../../build/hierarchical-lod-20261023/triangle_frontier_decoder.wasm',import.meta.url));
let requests=0;
const context=vm.createContext({self:{},Map,Uint8Array,Uint16Array,Float32Array,ArrayBuffer,DataView,URLSearchParams,
 fetch:async url=>{assert.ok(url.startsWith('/hierarchy/'));const value=entries.get(url.slice('/hierarchy/'.length));assert.ok(value,url);requests++;const bytes=Buffer.from(value.payload,'base64');return {ok:true,arrayBuffer:async()=>Uint8Array.from(bytes).buffer};},
 __decoder:await createTfcDecoder(wasm)});
vm.runInContext(await readFile(new URL('./hierarchy-tile.js',import.meta.url),'utf8'),context);
vm.runInContext('hierarchyDecoder=Promise.resolve(__decoder)',context);
function geometry(bytes){
 const v=new DataView(bytes.buffer,bytes.byteOffset,bytes.byteLength),ynt=v.getUint32(0,true)===0x32544e59,nv=v.getUint32(ynt?8:4,true),nf=v.getUint32(ynt?12:8,true);
 const offset=ynt?16:12;
 const points=Array.from({length:nv},(_,i)=>Array.from({length:3},(_,k)=>v.getFloat32(offset+i*12+k*4,true)).join(','));
 const count=ynt?nf:nf*3,indices=Array.from({length:count},(_,i)=>ynt?v.getUint16(offset+nv*12+i*2,true):v.getUint32(offset+nv*12+i*4,true));
 const faces=[];for(let i=0;i<count;i+=3){const f=indices.slice(i,i+3).map(id=>points[id]);const rotations=[f.join('|'),[f[1],f[2],f[0]].join('|'),[f[2],f[0],f[1]].join('|')];faces.push(rotations.sort()[0]);}
 return {points:points.sort(),faces:faces.sort()};
}
const network={bytes:0};
for(const item of fixture){
 const [q,x,y]=item.key.split('/').map(Number);
 const actual=await context.self.hierarchyGeometry(`${q-3}/${x}/${y}`,network);
 const expected=geometry(inflateSync(Buffer.from(item.reference,'base64')));
 assert.ok(JSON.stringify(geometry(actual.raw))===JSON.stringify(expected),`${item.key}の頂点または向き付き面が不一致`);
 assert.equal(actual.dependencies.length,Math.max(...fixture.map(v=>Number(v.key.split('/')[0])))-q+1);
}
assert.equal(requests,fixture.length,'共有する親タイルを二重取得してはいけません');
assert.equal(network.bytes,fixture.reduce((sum,v)=>sum+Buffer.from(v.payload,'base64').length,0));
console.log(`${fixture.length}タイルの頂点・向き付き面・親依存・一度だけの取得を確認`);
