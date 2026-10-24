// 新形式の数値復元はWASM、依存取得とBLOBヘッダーの検査はJavaScriptで行う。
export async function createXYZDecoder(binary){
 const {instance}=await WebAssembly.instantiate(binary,{}),e=instance.exports,base=Number(e.__heap_base.value),MAX=512*1048576;
 const align=n=>Math.ceil(n/8)*8;
 function ensure(total){if(!Number.isSafeInteger(total)||total>MAX)throw Error('WASM作業領域の上限を超えました');if(total>e.memory.buffer.byteLength)e.memory.grow(Math.ceil((total-e.memory.buffer.byteLength)/65536))}
 function decodeTFC(src,count,nv){if(!(src instanceof Uint8Array)||!Number.isSafeInteger(count)||count<0||!Number.isSafeInteger(nv)||nv<1||nv>4294967295||count>src.length-1)throw Error('TFC2の引数が不正です');const out=align(base+src.length),total=out+count*12;ensure(total);new Uint8Array(e.memory.buffer,base,src.length).set(src);const code=e.tfc_decode(base,src.length,out,count,BigInt(nv));if(code)throw Error('WASM TFC2入力不正 '+code);return new Uint32Array(e.memory.buffer,out,count*3).slice()}
 async function decodeYMV(data,z,x,y,resolve){const v=new DataView(data.buffer,data.byteOffset,data.byteLength);if(data.length<32||v.getUint32(0,true)!==0x34564d59||v.getUint16(4,true)!==4)throw Error('YMV4形式');const mode=v.getUint16(6,true),count=v.getUint32(8,true),d=v.getUint32(12,true),r=v.getUint32(16,true),a=v.getUint32(20,true),p=v.getUint32(24,true);if(count!==r+a||d>256||v.getUint32(28,true)||data.length!==32+12*d+p+6*a||!Number.isInteger(z)||z<6||z>14)throw Error('YMV4ヘッダー');
 const parents=await Promise.all(Array.from({length:d},(_,i)=>{const at=32+12*i;const zz=v.getUint32(at,true);if(zz!==z-1)throw Error('親レベル');return resolve(zz,v.getInt32(at+4,true),v.getInt32(at+8,true)).then(t=>t.points)}));
 const refs=align(base+data.length),table=align(refs+r*4),sizes=table+d*4,out=align(sizes+d*4);let end=out+count*24;const addresses=[];for(const parent of parents){if(!(parent instanceof Float64Array)||parent.length%3)throw Error('親頂点配列');addresses.push(end);end+=parent.byteLength}ensure(end);
 new Uint8Array(e.memory.buffer,base,data.length).set(data);
 const code=e.refs_decode(base+32+12*d,p,mode,r,d,refs);if(code)throw Error('WASM親参照入力不正 '+code);
 const tables=new Uint32Array(e.memory.buffer,table,d),counts=new Uint32Array(e.memory.buffer,sizes,d);
 for(let i=0;i<d;i++){tables[i]=addresses[i];counts[i]=parents[i].length/3;new Float64Array(e.memory.buffer,addresses[i],parents[i].length).set(parents[i])}
 if(e.vertices_refs(refs,r,table,sizes,out))throw Error('WASM親参照範囲');e.vertices_new(base+32+12*d+p,a,x,y,2**(14-z),out+r*24);
 return new Float64Array(e.memory.buffer,out,count*3).slice();
 }
 return {decodeTFC,decodeYMV};
}
