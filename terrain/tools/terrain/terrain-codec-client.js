/* 比較用の実ファイルをブラウザーで復元する。サーバーでの展開は行わない。 */
'use strict';
window.TerrainCodecs=(()=>{
 const files=new Map(),meshes=new Map(),hierarchies=new Map(),heights=new Map();let reportPromise,meshPromise,lercPromise;const heightAstcPackets=new Map();let heightAstcReport;
 const report=()=>reportPromise||(reportPromise=fetch('/codecs/report').then(r=>r.json()));
 async function bytes(url,onBytes){if(!files.has(url)){const response=await fetch(url);if(!response.ok)throw Error('圧縮データを取得できません: '+url);const data=new Uint8Array(await response.arrayBuffer());onBytes?.(data.length);files.set(url,data);}return files.get(url);}
 function packet(data,magic){if(data.length<8||new TextDecoder().decode(data.subarray(0,4))!==magic)throw Error('圧縮データの形式が不正です');const n=new DataView(data.buffer,data.byteOffset,data.byteLength).getUint32(4,true);if(n>data.length-8)throw Error('圧縮データの見出しが不足しています');return {header:JSON.parse(new TextDecoder().decode(data.subarray(8,8+n))),body:data.subarray(8+n)};}
 const zstd=data=>self.fzstd.decompress(data);
 async function meshDecoder(){if(!meshPromise)meshPromise=import('/codec-vendor/meshopt_decoder.mjs').then(async m=>{await m.MeshoptDecoder.ready;return m.MeshoptDecoder;});return meshPromise;}
 async function lerc(){if(!lercPromise)lercPromise=new Promise((resolve,reject)=>{const script=document.createElement('script');script.src='/codec-vendor/LercDecode.js';script.onload=resolve;script.onerror=()=>reject(Error('LERC の読み込みに失敗しました'));document.head.append(script);}).then(async()=>{await self.Lerc.load({locateFile:()=>'/codec-vendor/lerc-wasm.wasm'});return self.Lerc;});return lercPromise;}
 function vertexDecode(decoder,body,count){if(!count)return new Uint16Array();const out=new Uint8Array(count*8);decoder.decodeVertexBuffer(out,count,8,body);return new Uint16Array(out.buffer);}
 function indexDecode(decoder,body,count,sequence=false){if(!count)return new Uint32Array();const out=new Uint8Array(count*4);decoder[sequence?'decodeIndexSequence':'decodeIndexBuffer'](out,count,4,body);return new Uint32Array(out.buffer);}
 function worldVertices(vertices,header){const out=new Float32Array(vertices.length/4*3);for(let i=0;i<vertices.length/4;i++){out[i*3]=header.origin[0]+(vertices[i*4]+(vertices[i*4+3]&255)*65536)*header.step;out[i*3+1]=header.origin[1]+(vertices[i*4+1]+(vertices[i*4+3]>>8)*65536)*header.step;out[i*3+2]=header.origin[2]+vertices[i*4+2]*header.step;}return out;}
 async function hierarchy(level,onBytes){
  if(hierarchies.has(level))return hierarchies.get(level);
  const r=await report(),row=r.meshes?.results.find(r=>r.level===level&&r.codec==='hierarchy');if(!row)throw Error('階層メッシュの計測データがありません');
  const data=await bytes(row.url,onBytes),decoder=await meshDecoder(),started=performance.now(),{header:h,body}=packet(zstd(data),'YMH1');
  const parent=h.parentLevel?await hierarchy(h.parentLevel,onBytes):{vertices:new Uint16Array(),indices:new Uint32Array()};
  if(parent.vertices.length/4!==h.previousVertexCount)throw Error('階層メッシュの親が違います');
  const added=vertexDecode(decoder,body.subarray(0,h.vertexBytes),h.newVertexCount),vertices=new Uint16Array(parent.vertices.length+added.length);vertices.set(parent.vertices);vertices.set(added,parent.vertices.length);
  let indices;
  if(h.indexMode==='full')indices=indexDecode(decoder,body.subarray(h.vertexBytes),h.indexCount);
  else{
   const removed=indexDecode(decoder,body.subarray(h.vertexBytes,h.vertexBytes+h.removedBytes),h.removedCount,true),deleted=new Uint8Array(parent.indices.length/3);
   for(const i of removed){if(i>=deleted.length)throw Error('階層メッシュの面が範囲外です');deleted[i]=1;}
   const faces=indexDecode(decoder,body.subarray(h.vertexBytes+h.removedBytes),h.addedIndexCount);indices=new Uint32Array(h.indexCount);let p=0;
   for(let i=0;i<deleted.length;i++)if(!deleted[i])for(let j=0;j<3;j++)indices[p++]=parent.indices[i*3+j];
   indices.set(faces,p);if(p+faces.length!==indices.length)throw Error('階層メッシュの面数が違います');
  }
  const result={vertices,indices,header:h,row,decodeMs:performance.now()-started};hierarchies.set(level,result);return result;
 }
 async function loadMesh(level,codec,onBytes){
  const key=level+'-'+codec;if(meshes.has(key))return meshes.get(key);
  const r=await report(),row=r.meshes?.results.find(r=>r.level===level&&r.codec===codec);if(!row)throw Error('圧縮メッシュの計測データがありません');
  let vertices,indices,h,decodeMs;
  if(codec==='hierarchy'){const result=await hierarchy(level,onBytes);({vertices,indices,header:h,decodeMs}=result);}
  else{const data=await bytes(row.url,onBytes),decoder=await meshDecoder(),started=performance.now(),p=packet(codec.endsWith('zstd')?zstd(data):data,'YMC1');h=p.header;
   if(h.vertexEncoding==='f32x3'){
    const target=new Uint8Array(h.vertexCount*12);decoder.decodeVertexBuffer(target,h.vertexCount,12,p.body.subarray(0,h.vertexBytes));indices=indexDecode(decoder,p.body.subarray(h.vertexBytes),h.indexCount);if(indices.some(i=>i>=h.vertexCount))throw Error('頂点が範囲外です');
    const result={points:new Float32Array(target.buffer),indices,bytes:row.bytes,fileBytes:row.bytes,heightError:0,decodeMs:performance.now()-started,row};meshes.set(key,result);return result;
   }
   vertices=vertexDecode(decoder,p.body.subarray(0,h.vertexBytes),h.vertexCount);indices=indexDecode(decoder,p.body.subarray(h.vertexBytes),h.indexCount);decodeMs=performance.now()-started;
  }
  if(indices.some(i=>i>=vertices.length/4))throw Error('圧縮メッシュの頂点が範囲外です');
  const result={points:worldVertices(vertices,h),indices,bytes:row.requiredBytes||row.bytes,fileBytes:row.bytes,heightError:row.heightMaxErrorMeters,decodeMs,row};meshes.set(key,result);return result;
 }
 async function decodeYmd(data){
  const h=new DataView(data.buffer,data.byteOffset,data.byteLength),rows=h.getUint16(4,true),cols=h.getUint16(6,true),step=h.getFloat32(8,true),base=h.getInt32(12,true),flags=data[16],wide=(flags&1)?4:2;
  if(new TextDecoder().decode(data.subarray(0,4))!=='YMD1'||rows*cols>4e6)throw Error('YMD1 の形式が不正です');
  const raw=await unzip(data.subarray(18),data[17]?'xz':'gzip'),v=new DataView(raw.buffer,raw.byteOffset,raw.byteLength),n=rows*cols,out=new Float32Array(n),q=new Int32Array(n);
  if(raw.length!==n*wide+((flags&2)?Math.ceil(n/8):0))throw Error('YMD1 の長さが不正です');
  for(let y=0;y<rows;y++)for(let x=0;x<cols;x++){const i=y*cols+x,d=wide===2?v.getInt16(i*2,true):v.getInt32(i*4,true);q[i]=d+(x?q[i-1]:0)+(y?q[i-cols]:0)-(x&&y?q[i-cols-1]:0);out[i]=(flags&2)&&(raw[n*wide+(i>>3)]&(1<<(i&7)))?NaN:(q[i]+base)*step;}
  return out;
 }
 function rememberHeight(key,value){heights.set(key,value);let total=[...heights.values()].reduce((s,h)=>s+h.values.byteLength,0);while(total>128*1024*1024&&heights.size>1){const key=heights.keys().next().value;total-=heights.get(key).values.byteLength;heights.delete(key);}return value;}
 const round=x=>x<0?-Math.round(-x):Math.round(x);
 function prediction(parent,x,y,w,h){const sx=x*(parent.width-1)/(w-1),sy=y*(parent.height-1)/(h-1),x0=Math.floor(sx),y0=Math.floor(sy),fx=sx-x0,fy=sy-y0;let sum=0,weight=0;
  for(const [dx,dy,k] of [[0,0,(1-fx)*(1-fy)],[1,0,fx*(1-fy)],[0,1,(1-fx)*fy],[1,1,fx*fy]]){const v=parent.values[Math.min(y0+dy,parent.height-1)*parent.width+Math.min(x0+dx,parent.width-1)];if(Number.isFinite(v)){sum+=v*k;weight+=k;}}
  return weight?round(sum/weight):0;
 }
 async function loadHeightASTC(level,codec,onBytes){
  const key=level+'-'+codec;if(heightAstcPackets.has(key))return heightAstcPackets.get(key);
  if(!heightAstcReport||heightAstcReport.state!=='complete')heightAstcReport=await(await fetch('/height-astc/report.json')).json();
  const row=heightAstcReport.results.find(r=>r.level===level&&r.codec===codec);if(!row)throw Error('この標高 ASTC はまだ生成されていません');
  const data=await bytes(row.url,onBytes),started=performance.now(),p=packet(data,'YHA1'),h=p.header,raw=zstd(p.body.subarray(0,h.astcBytes)),mask=zstd(p.body.subarray(h.astcBytes));
  if(raw.length!==h.gpuBytes||mask.length!==h.width*h.height||h.width*h.height>2e7)throw Error('標高 ASTC の長さが不正です');
  const result={header:h,raw,mask,row,bytes:row.bytes,decodeMs:performance.now()-started};heightAstcPackets.set(key,result);return result;
 }
 async function decodeHeightASTC(level,codec,onBytes){
  const key=level+'-'+codec;if(heights.has(key))return heights.get(key);
  const p=await loadHeightASTC(level,codec,onBytes),h=p.header,started=performance.now(),image=await createImageBitmap(new Blob([await bytes(p.row.previewUrl,onBytes)],{type:'image/png'}),{colorSpaceConversion:'none',premultiplyAlpha:'none'});
  if(image.width!==h.width||image.height!==h.height)throw Error('標高 ASTC の比較画像の寸法が違います');
  const c=new OffscreenCanvas(h.width,h.height),ctx=c.getContext('2d');ctx.drawImage(image,0,0);image.close();const pixels=ctx.getImageData(0,0,h.width,h.height).data,values=new Float32Array(h.width*h.height);
  for(let y=0;y<h.height;y++)for(let x=0;x<h.width;x++){const i=y*h.width+x,t=h.tiles[Math.floor(y/h.core)*h.columns+Math.floor(x/h.core)];values[i]=p.mask[i]?t.min+pixels[i*4]/255*t.range:NaN;}
  return rememberHeight(key,{values,width:h.width,height:h.height,bytes:p.bytes,decodeMs:performance.now()-started+p.decodeMs,row:p.row});
 }
 async function loadHeight(level,codec,onBytes){
  if(codec.startsWith('height-astc'))return decodeHeightASTC(level,codec,onBytes);
  const key=level+'-'+codec;if(heights.has(key))return heights.get(key);
  const r=await report(),row=r.heights?.results.find(r=>r.level===level&&r.codec===codec);if(!row)throw Error('圧縮標高の計測データがありません');
  const data=await bytes(row.url,onBytes),started=performance.now();let header,values;
  if(codec==='residual-zstd'||codec==='delta-zstd'){
   const p=packet(data,'YHR1');header=p.header;const raw=zstd(p.body),view=new DataView(raw.buffer,raw.byteOffset,raw.byteLength),n=header.width*header.height,q=new Int32Array(n);values=new Float32Array(n);
   if(n>2e7||raw.length!==n*4+(header.missing?n:0))throw Error('階層標高の長さが不正です');
   const parent=header.parentLevel?await loadHeight(header.parentLevel,'residual-zstd',onBytes):null;
   for(let y=0;y<header.height;y++)for(let x=0;x<header.width;x++){const i=y*header.width+x;q[i]=view.getInt32(i*4,true)+(x?q[i-1]:0)+(y?q[i-header.width]:0)-(x&&y?q[i-header.width-1]:0);values[i]=header.missing&&!raw[n*4+i]?NaN:q[i]+(parent?prediction(parent,x,y,header.width,header.height):0);}
  }else{
   const p=packet(data,'YHB1');header=p.header;if(header.width*header.height>2e7)throw Error('標高が大きすぎます');values=new Float32Array(header.width*header.height);values.fill(NaN);let offset=0;
   const l=header.codec==='lerc'?await lerc():null;
   for(const tile of header.tiles){const body=p.body.subarray(offset,offset+tile.bytes);offset+=tile.bytes;let decoded;
    if(l){const block=l.decode(body.buffer.slice(body.byteOffset,body.byteOffset+body.byteLength));decoded=block.pixels[0];if(block.mask)for(let i=0;i<decoded.length;i++)if(!block.mask[i])decoded[i]=NaN;}
    else decoded=await decodeYmd(body);
    for(let y=0;y<tile.height;y++)values.set(decoded.subarray(y*tile.width,(y+1)*tile.width),(tile.y+y)*header.width+tile.x);
   }
  }
  return rememberHeight(key,{values,width:header.width,height:header.height,bytes:row.requiredBytes||row.bytes,fileBytes:row.bytes,decodeMs:performance.now()-started,row});
 }
 function ktxPayload(data){
  if(data.length<104||new TextDecoder().decode(data.subarray(1,7))!=='KTX 20')throw Error('KTX2 の識別子が不正です');const v=new DataView(data.buffer,data.byteOffset,data.byteLength),offset=Number(v.getBigUint64(80,true)),length=Number(v.getBigUint64(88,true)),scheme=v.getUint32(44,true);
  if(offset+length>data.length||![0,2].includes(scheme))throw Error('KTX2 のレベルが不正です');const payload=data.subarray(offset,offset+length);return scheme===2?zstd(payload):payload;
 }
 return {report,bytes,loadMesh,loadHeight,loadHeightASTC,ktxPayload};
})();
