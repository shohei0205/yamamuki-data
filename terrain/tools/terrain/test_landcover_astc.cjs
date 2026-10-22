const assert=require('node:assert/strict');require('./landcover-astc.js');
const {parse,upload}=globalThis.LandcoverAstc;
function example(){const n=8,count=4,buffer=new ArrayBuffer(12+count*(12+16)),view=new DataView(buffer);new Uint8Array(buffer).set(Buffer.from('LCA1'));view.setUint32(4,n,true);view.setUint32(8,count,true);let at=12;for(const size of [8,4,2,1]){view.setUint32(at,size,true);view.setUint32(at+4,size,true);view.setUint32(at+8,16,true);at+=28;}return buffer;}
const bytes=example(),levels=parse(bytes,8);assert.deepEqual(levels.map(m=>m.size),[8,4,2,1]);
assert.throws(()=>parse(bytes.slice(0,-1),8));assert.throws(()=>parse(bytes,16));assert.throws(()=>parse(bytes,8,true));
const malformed=example();new DataView(malformed).setUint32(20,15,true);assert.throws(()=>parse(malformed,8));
const calls=[],texture={},gl={TEXTURE5:5,TEXTURE_2D:2,NO_ERROR:0,createTexture:()=>texture,activeTexture:()=>{},bindTexture:()=>{},compressedTexImage2D:(...a)=>calls.push(a),getError:()=>0,deleteTexture:()=>{throw Error('成功したテクスチャを削除しました')}};
assert.equal(upload(gl,{COMPRESSED_RGBA_ASTC_8x8_KHR:0x93b7},levels),texture);assert.equal(calls.length,4);for(let i=0;i<4;i++){assert.equal(calls[i][1],i);assert.equal(calls[i][2],0x93b7);assert.equal(calls[i][6].byteLength,16);}
let deleted=false;gl.getError=()=>1280;gl.deleteTexture=()=>deleted=true;assert.throws(()=>upload(gl,{COMPRESSED_RGBA_ASTC_8x8_KHR:0x93b7},levels));assert.equal(deleted,true);
console.log('ASTCミップ検証・全ミップの直接アップロード呼び出し・失敗時解放に成功（モックGL）');
