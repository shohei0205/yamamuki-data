// Workerと数値検証で共有するWASMのメモリー入出力。
export async function createVariableMeshWasm(bytes){
 const {instance}=await WebAssembly.instantiate(bytes,{}),e=instance.exports,base=Number(e.__heap_base.value);
 return function build(tiles,join=true){
  const start=performance.now();let cursor=base;const alloc=(bytes,alignment=8)=>{cursor=Math.ceil(cursor/alignment)*alignment;const p=cursor;cursor+=bytes;return p;};
  const count=tiles.length;if(!count)throw Error('タイルが空です');const meta=alloc(count*16*8),ranges=alloc(count*2*4),capacity=Math.max(16,count*16),neighbors=alloc(capacity*4),parts=alloc(count*4*4),dataPointers=[];let points=0,indices=0;
  for(const t of tiles){const i=t.info;if(!Number.isInteger(i.n)||i.n<1||i.n>512||t.data.length!==(i.nx+1)*(i.ny+1)*3)throw Error('不正な表示タイルです');dataPointers.push(alloc(t.data.byteLength));points+=(i.n+1)**2;indices+=i.n*i.n*6;}
  const vertices=alloc(points*5*4),index=alloc(indices*4);if(cursor>e.memory.buffer.byteLength)e.memory.grow(Math.ceil((cursor-e.memory.buffer.byteLength)/65536));
  const metadata=new Float64Array(e.memory.buffer,meta,count*16);
  tiles.forEach((t,k)=>{const i=t.info;metadata.set([...i.bounds,i.side,i.error,...i.fieldBounds,i.n,i.nx,i.ny,dataPointers[k],0,0],k*16);new Float32Array(e.memory.buffer,dataPointers[k],t.data.length).set(t.data);});
  const adjacencyStart=performance.now(),used=e.prepare_neighbors(count,meta,ranges,neighbors,capacity);if(used<0)throw Error('隣接表の容量を超えました');const mathStart=performance.now(),code=e.build_meshes(count,meta,ranges,neighbors,join?1:0,vertices,index,parts);if(code)throw Error('メッシュ組み立て失敗: '+code);const copyStart=performance.now();
  const result={vertices:new Float32Array(e.memory.buffer,vertices,points*5).slice().buffer,indices:new Uint32Array(e.memory.buffer,index,indices).slice().buffer,parts:new Uint32Array(e.memory.buffer,parts,count*4).slice(),stats:{backend:'WASM Worker',tiles:count,points,adjacencyMs:mathStart-adjacencyStart,mathMs:copyStart-mathStart,copyMs:0,totalMs:0,memoryBytes:e.memory.buffer.byteLength}};result.stats.copyMs=performance.now()-copyStart;result.stats.totalMs=performance.now()-start;return result;
 };
}
