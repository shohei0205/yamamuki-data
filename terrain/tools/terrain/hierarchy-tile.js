/* 親頂点差分とTFC1はブラウザーで復元する。サーバーでの形状変換は行わない。 */
'use strict';
const hierarchyTiles=new Map();
let hierarchyDecoder;
async function hierarchyDecode(){
 if(!hierarchyDecoder)hierarchyDecoder=(async()=>{const module=await import('./triangle_frontier_wasm.mjs');const response=await fetch('/triangle_frontier_decoder.wasm');if(!response.ok)throw Error('TFC1復元器を取得できません');return module.createTfcDecoder(await response.arrayBuffer());})();
 return hierarchyDecoder;
}
function hierarchyRestore(q,x,y,network){
 const key=`${q}/${x}/${y}`;
 if(hierarchyTiles.has(key))return hierarchyTiles.get(key);
 const promise=(async()=>{
  if(!Number.isInteger(q)||q<4||q>12||!Number.isSafeInteger(x)||!Number.isSafeInteger(y))throw Error('階層タイル番号が不正です');
  const response=await fetch(`/hierarchy/${key}`);if(!response.ok)throw Error(`階層タイルを取得できません ${key}`);
  const buffer=await response.arrayBuffer();network.bytes+=buffer.byteLength;
  const v=new DataView(buffer);if(buffer.byteLength<56||v.getUint32(0,true)!==0x31444859)throw Error('階層タイル形式が不正です');
  if(v.getInt32(4,true)!==q||v.getInt32(8,true)!==x||v.getInt32(12,true)!==y)throw Error('階層タイルの内容が一致しません');
  const pq=v.getInt32(16,true),px=v.getInt32(20,true),py=v.getInt32(24,true),nv=v.getUint32(28,true),nf=v.getUint32(32,true),inherited=v.getUint32(36,true),refs=v.getUint32(40,true),vertices=v.getUint32(44,true),indices=v.getUint32(48,true),borders=v.getUint32(52,true),stride=64*2**(q-4);
  if(nv<4||nv>65535||nf>2000000||inherited>nv||refs!==inherited*2||vertices!==(nv-inherited)*6||buffer.byteLength!==56+refs+vertices+indices+borders)throw Error('階層タイルの個数・長さが不正です');
  const p=new Float32Array(nv*3),dependencies=new Map();
  if(pq>=0){if(pq!==q+1||pq>12||px!==Math.floor(x/2)||py!==Math.floor(y/2))throw Error('親タイルの参照が不正です');
   const parent=await hierarchyRestore(pq,px,py,network);for(const pair of parent.dependencies)dependencies.set(...pair);
   let previous=-1;for(let i=0;i<inherited;i++){const id=v.getUint16(56+i*2,true);if(id<=previous||id>=parent.points.length/3)throw Error('親頂点の参照が不正です');previous=id;p.set([parent.points[id*3]-(x-px*2)*stride*31.25,parent.points[id*3+1]-(y-py*2)*stride*31.25,parent.points[id*3+2]],i*3);}
  }else if(inherited)throw Error('最上位に親頂点の参照があります');
  for(let i=inherited;i<nv;i++){const at=56+refs+(i-inherited)*6;p.set([v.getUint16(at,true)*31.25,v.getUint16(at+2,true)*31.25,v.getInt16(at+4,true)/4],i*3);}
  for(let i=0;i<nv;i++)if(p[i*3]<0||p[i*3+1]<0||p[i*3]>stride*31.25||p[i*3+1]>stride*31.25)throw Error('階層タイルの頂点が範囲外です');
  const decode=await hierarchyDecode(),faces=decode(new Uint8Array(buffer,56+refs+vertices,indices),nf*3,nv);
  dependencies.set(`${q-3}/${x}/${y}/geometry`,refs+vertices+indices+borders);
  return {points:p,faces,n:stride+1,inherited,dependencies};
 })();
 hierarchyTiles.set(key,promise);return promise.catch(error=>{hierarchyTiles.delete(key);throw error;});
}
self.hierarchyGeometry=async function(key,network){
 const [level,x,y]=key.split('/').map(Number),mesh=await hierarchyRestore(level+3,x,y,network),nv=mesh.points.length/3,ni=mesh.faces.length;
 const out=new Uint8Array(16+nv*12+ni*2),v=new DataView(out.buffer);out.set([89,78,84,50]);v.setUint16(4,mesh.n,true);v.setUint16(6,mesh.n,true);v.setUint32(8,nv,true);v.setUint32(12,ni,true);
 for(let i=0;i<mesh.points.length;i++)v.setFloat32(16+i*4,mesh.points[i],true);for(let i=0;i<ni;i++)v.setUint16(16+nv*12+i*2,mesh.faces[i],true);
 return {raw:out,dependencies:[...mesh.dependencies]};
};
