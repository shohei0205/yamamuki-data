import fs from 'node:fs';import assert from 'node:assert/strict';import {createRequire} from 'node:module';import {createXYZDecoder} from './xyz_wasm.mjs';
const require=createRequire(import.meta.url),js=require('./xyz_viewer.js'),wasm=await createXYZDecoder(fs.readFileSync('build/xyz_decoder.wasm'));let checked=0;
for(const fixture of JSON.parse(fs.readFileSync(process.argv[2],'utf8'))){const cache=new Map();async function resolve(z,x,y){const key=[z,x,y].join(',');if(cache.has(key))return cache.get(key);const data=new Uint8Array(Buffer.from(fixture.rows[key].vertices,'base64'));const promise=wasm.decodeYMV(data,z,x,y,resolve).then(points=>({points}));cache.set(key,promise);return promise}
for(const t of fixture.targets){const points=(await resolve(...t.key)).points;assert.deepEqual(Array.from(points),t.points.flat());const data=new Uint8Array(Buffer.from(fixture.rows[t.key.join(',')].triangles,'base64')),view=new DataView(data.buffer,data.byteOffset,data.byteLength),count=view.getUint32(12,true),length=view.getUint32(16,true),stream=data.subarray(24,24+length);assert.deepEqual(wasm.decodeTFC(stream,count,points.length/3),js.decodeTFC(stream,count,points.length/3));checked++}}
function varint(v){const a=[];while(v>=128){a.push(Number(v%128n)+128);v/=128n}a.push(Number(v));return a}
const values=[4000000000n,3999999999n,3999999998n];let prev=0n;const bytes=[194,240];for(const v of values){const d=v-prev;bytes.push(...varint(d>=0n?d*2n:-d*2n-1n));prev=v}
assert.deepEqual(Array.from(wasm.decodeTFC(new Uint8Array(bytes),1,4294967295)),values.map(Number));
for(const bad of [new Uint8Array(),new Uint8Array([193]),new Uint8Array([194,255]),new Uint8Array([194,240,128])])assert.throws(()=>wasm.decodeTFC(bad,1,10));
assert.throws(()=>wasm.decodeTFC(new Uint8Array([194,240,0,0,0]),1,10));
console.log(checked+' WASM region/level fixtures match JS and Python; u32 large indices and malformed streams passed');
