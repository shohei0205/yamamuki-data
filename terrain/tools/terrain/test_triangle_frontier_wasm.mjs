import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import {encodeTriangles,decodeTriangles} from './triangle_frontier_codec.mjs';
import {createTfcDecoder} from './triangle_frontier_wasm.mjs';
const root=new URL('../../build/hierarchical-lod-20261023/',import.meta.url);
const binary=fs.readFileSync(new URL('triangle_frontier_decoder.wasm',root));
const decode=await createTfcDecoder(binary);
test('外部関数への依存がない',()=>{
 assert.deepEqual(WebAssembly.Module.imports(new WebAssembly.Module(binary)),[]);
});
test('全地域のTFC1ストリームとJavaScript版が完全一致',()=>{
 const rows=JSON.parse(fs.readFileSync(new URL('triangle-codec-blobs.json',root)));
 for(const row of rows)for(const key of ['frontierOriginal','frontierGreedy','frontierCache']){
  const blob=Buffer.from(row.encoded[key],'base64');
  assert.deepEqual(decode(blob,row.indexCount),decodeTriangles(blob,row.indexCount));
 }
});
test('入力の破損と実タイルの頂点数超過を拒否',()=>{
 const blob=encodeTriangles(new Uint16Array([65533,65534,65535,65535,65534,0]));
 for(let i=0;i<blob.length;i++)assert.throws(()=>decode(blob.subarray(0,i),6));
 for(const bad of [new Uint8Array([...blob,0]),new Uint8Array([0xc1,241]),new Uint8Array([0xc1,0]),new Uint8Array([0xc1,240,128,128,8,0,0])])assert.throws(()=>decode(bad,3));
 assert.throws(()=>decode(blob,2));assert.throws(()=>decode(blob,6000003));assert.throws(()=>decode(blob,6,65535));
 assert.deepEqual([...decode(new Uint8Array([0xc1]),0)],[]);
 assert.deepEqual([...decode(encodeTriangles(new Uint16Array([0,1,2])),3)],[0,1,2]);
});
test('メモリ拡張後と失敗後も再利用できる',()=>{
 const a=new Uint16Array(150000);for(let i=0;i<a.length;i++)a[i]=i%3;
 assert.deepEqual(decode(encodeTriangles(a,false),a.length),a);
 assert.throws(()=>decode(new Uint8Array([0]),3));
 assert.deepEqual([...decode(encodeTriangles(new Uint16Array([1,2,3])),3)],[1,2,3]);
});
