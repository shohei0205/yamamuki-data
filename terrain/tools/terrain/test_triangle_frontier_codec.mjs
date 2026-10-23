import {test} from 'node:test';
import assert from 'node:assert/strict';
import {encodeTriangles,decodeTriangles} from './triangle_frontier_codec.mjs';
function canonical(a){const out=[];for(let i=0;i<a.length;i+=3){const f=[...a.subarray(i,i+3)],k=f.indexOf(Math.min(...f));out.push([f[k],f[(k+1)%3],f[(k+2)%3]].join(','));}return out.sort();}
test('空・非連結・uint16上限・巡回開始点',()=>{
 for(const a of [[],[0,1,2],[0,1,2,2,1,3,4,5,6],[65533,65534,65535],[2,0,1,1,3,2]])for(const reorder of [false,true]){
  const src=new Uint16Array(a);assert.deepEqual(canonical(decodeTriangles(encodeTriangles(src,reorder),src.length)),canonical(src));
 }
});
test('格子地形・頂点の再番号付け・面順の逆転',()=>{
 let seed=731;const random=n=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;return seed%n;};
 for(const width of [3,8,24]){
  const ids=Array.from({length:width*width},(_,i)=>i);for(let i=ids.length-1;i>0;i--){const j=random(i+1);[ids[i],ids[j]]=[ids[j],ids[i]];}
  const fs=[];for(let y=0;y<width-1;y++)for(let x=0;x<width-1;x++){const a=y*width+x;fs.push([a,a+1,a+width],[a+width,a+1,a+width+1]);}
  for(const reverse of [false,true]){const src=new Uint16Array((reverse?fs.slice().reverse():fs).flat().map(i=>ids[i]));for(const reorder of [false,true]){const blob=encodeTriangles(src,reorder);assert.deepEqual(canonical(decodeTriangles(blob,src.length)),canonical(src));}}
 }
});
test('壊れた入力を拒否',()=>{
 const src=new Uint16Array([0,1,2,2,1,3]),blob=encodeTriangles(src,true);
 for(let i=0;i<blob.length;i++)assert.throws(()=>decodeTriangles(blob.subarray(0,i),src.length));
 assert.throws(()=>decodeTriangles(new Uint8Array([...blob,0]),src.length));
 assert.throws(()=>decodeTriangles(new Uint8Array([0xc1,241]),3));
 assert.throws(()=>decodeTriangles(new Uint8Array([0xc1,0]),3));
 assert.throws(()=>decodeTriangles(new Uint8Array([0xc1,240,128,128,8,0,0]),3));
 assert.throws(()=>decodeTriangles(blob,2));assert.throws(()=>decodeTriangles(blob,6000003));
});
