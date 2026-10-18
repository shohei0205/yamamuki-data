/* サーバーは SELECT と中継だけを行い、以下の処理はすべて端末側で行う。 */
'use strict';
importScripts('/vendor/xz-decompress-0.2.3.min.js','/vendor/polygon-clipping-0.15.7.min.js','/vendor/earcut-3.2.4.min.js','/terrain-client-core.js');
let serial=0,metadataPromise,peaksPromise,cacheBytes=0;
const tileCache=new Map(),payloadCache=new Map(),overviewBlobs=new Map();
const OVERVIEW_LEVEL=4;
function cacheTile(key,value){
 const memory=value.points.length*24+value.indices.length*8;
 if(tileCache.has(key))cacheBytes-=tileCache.get(key).memory;
 const cached={...value,memory};tileCache.set(key,cached);cacheBytes+=memory;
 while(cacheBytes>64*1024**2&&tileCache.size>1){const oldest=tileCache.keys().next().value;cacheBytes-=tileCache.get(oldest).memory;tileCache.delete(oldest);}
 return cached;
}
function progress(key,label,values){postMessage({type:'progress',key,label,progress:values});}
async function getBytes(url,label){
 const key='sqlite-fetch-'+(++serial);progress(key,label,{phase:'waiting',received:0,compressed:null});
 try{
  const response=await fetch(url,{signal:AbortSignal.timeout(120000)});
  if(!response.ok){let detail;try{detail=await response.json();}catch{}throw Error(detail?.error||'保存データを取得できませんでした');}
  const length=response.headers.get('content-length'),compressed=length===null?null:Number(length);
  progress(key,label,{phase:'receiving',compressed});
  const chunks=[];let received=0;const reader=response.body.getReader();
  for(;;){const {done,value}=await reader.read();if(done)break;chunks.push(value);received+=value.length;progress(key,label,{received});}
  const bytes=new Uint8Array(received);let cursor=0;for(const chunk of chunks){bytes.set(chunk,cursor);cursor+=chunk.length;}
  postMessage({type:'received',key,bytes:Number.isFinite(compressed)?compressed:received});return bytes;
 }finally{postMessage({type:'finish-progress',key});}
}
function unpack(bytes){
 if(bytes.length<8)throw Error('SQLite 応答のヘッダーが不足しています');
 const view=new DataView(bytes.buffer,bytes.byteOffset,bytes.byteLength),length=view.getUint32(4,true);
 if(view.getUint32(0,true)!==0x31515359||length>bytes.length-8)throw Error('SQLite 応答の形式が不正です');
 const rows=JSON.parse(new TextDecoder().decode(bytes.subarray(8,8+length)));let cursor=8+length;
 for(const row of rows){if(!Number.isSafeInteger(row.bytes)||row.bytes<0||cursor+row.bytes>bytes.length)throw Error('SQLite 応答の長さが不正です');row.body=bytes.subarray(cursor,cursor+row.bytes);cursor+=row.bytes;}
 if(cursor!==bytes.length)throw Error('SQLite 応答が余っています');return rows;
}
async function metadata(){if(!metadataPromise)metadataPromise=getBytes('/sqlite/metadata','SQLite の目録').then(b=>JSON.parse(new TextDecoder().decode(b))).catch(error=>{metadataPromise=null;throw error;});return metadataPromise;}
async function peaks(){if(!peaksPromise)peaksPromise=getBytes('/sqlite-peaks','山頂名').then(b=>JSON.parse(new TextDecoder().decode(b))).catch(error=>{peaksPromise=null;throw error;});return peaksPromise;}
function keyOf(query){const params=new URLSearchParams(query);params.delete('stage');params.delete('format');params.delete('fullscreen');params.sort();return params.toString();}
let overviewPromise;
async function overview(query,meta,task){
 if(!overviewPromise)overviewPromise=(async()=>{
  const records=unpack(await getBytes('/sqlite/overview?level='+OVERVIEW_LEVEL,`全国の地形（レベル ${OVERVIEW_LEVEL}）`));
  if(!records.length)return [];
  for(const row of records)overviewBlobs.set([row.z,row.x,row.y].join(','),row.body);
  return records.map(row=>[row.z,row.x,row.y]);
 })().catch(error=>{overviewPromise=null;throw error;});
 const tiles=await overviewPromise,lat=Number(query.get('lat')),lon=Number(query.get('lon')),size=Number(query.get('size')),rows=new Map();
 if(!tiles.length)return terrain(query,meta,task);
 for(let i=0;i<tiles.length;i++){
  const key=tiles[i].join(','),body=overviewBlobs.get(key);
  rows.set(key,{...await decodeMesh(body),bytes:body.length});
  progress(task,'全国の地形',{phase:'decoding',detail:`展開 ${i+1} / ${tiles.length} タイル`});
 }
 const cameraPlan=selection(lat,lon,size,query.has('z')?Number(query.get('z')):null,null,query.get('levelMode')==='manual',meta.terrain);
 const plan={tiles,zoom:OVERVIEW_LEVEL,center:cameraPlan.center,levels:[...meta.terrain.levels].sort((a,b)=>a-b),x0:Math.min(...tiles.map(t=>t[1])),y0:Math.min(...tiles.map(t=>t[2])),x1:Math.max(...tiles.map(t=>t[1])),y1:Math.max(...tiles.map(t=>t[2]))};
 const result=assembleTerrain(plan,rows,meta.terrain,lat,lon,size,null,false);
 const meters=2*PI*R*Math.cos(cameraPlan.center[1]*PI/180)/(256*2**geometryZoom(cameraPlan.zoom,meta.terrain));
 result.span=Math.max(cameraPlan.x1-cameraPlan.x0+1,cameraPlan.y1-cameraPlan.y0+1)*256*meters;
 result.initialCameraDistance=result.span*Math.SQRT2*.8;
 const [clon,clat]=result.projectionCenter,sx=PI/180*R*Math.cos(clat*PI/180),sy=PI/180*R;
 const height=new TerrainSurface(result.mesh,result.indices).height((lon-clon)*sx,(lat-clat)*sy);
 result.initialViewHeight=Number.isFinite(height)?height:0;
 if(meta.normals){
  const normalRows=unpack(await getBytes('/sqlite/normals','全国の陰影マップ'));result.normalMaps=[];
  let packed=0,decoded=0;
  for(const row of normalRows){
   if(row.z!==4||row.width!==row.height||row.width>2048)throw Error('陰影マップの形式が不正です');
   const pixels=await unzip(row.body,'gzip');if(pixels.length!==row.width*row.height*2)throw Error('陰影マップの長さが不正です');
   result.normalMaps.push({x:row.x,y:row.y,z:row.z,width:row.width,height:row.height,pixels});packed+=row.body.length;decoded+=pixels.length;
  }
  result.transferSize={...result.transferSize,compressed:result.transferSize.compressed+packed,decoded:result.transferSize.decoded+decoded};
 }
 result.nationalBaseSize={...result.transferSize,level:OVERVIEW_LEVEL};
 result.coverage.overviewFallback=true;result.coverage.levelMode='fallback';
 return result;
}
async function terrain(query,meta,task){
 const key=keyOf(query);if(payloadCache.has(key))return payloadCache.get(key);
 const lat=Number(query.get('lat')),lon=Number(query.get('lon')),size=Number(query.get('size')),fixed=query.get('levelMode')==='manual';
 if(query.has('levelMode')&&!['auto','manual'].includes(query.get('levelMode')))throw Error('レベルの選択方法が不正です');
 const requested=query.has('z')?Number(query.get('z')):null;
 if(requested!==null&&(!Number.isInteger(requested)||requested<4||requested>14))throw Error('地形レベルが不正です');
 const view=query.has('tilt')?['tilt','bearing','distance'].map(k=>Number(query.get(k))):null;
 if(view&&(!view.every(Number.isFinite)||view[0]<0||view[0]>90||view[1]<0||view[1]>360||view[2]<=0))throw Error('視点が不正です');
 const plan=selection(lat,lon,size,requested,view,fixed,meta.terrain),rows=new Map(),missing=[];
 for(const tile of plan.tiles){const key=tile.join(',');if(tileCache.has(key)){const row=tileCache.get(key);tileCache.delete(key);tileCache.set(key,row);rows.set(key,row);}else if(overviewBlobs.has(key)){const body=overviewBlobs.get(key);rows.set(key,cacheTile(key,{...await decodeMesh(body),bytes:body.length}));}else missing.push(tile);}
 let downloaded=[];
 if(missing.length)downloaded=unpack(await getBytes('/sqlite/mesh?'+new URLSearchParams({tiles:missing.map(t=>t.join(',')).join(';')}),'地形タイル'));
 progress(task,'地形',{phase:'decoding',received:0,compressed:null});
 let outlines=new Map();
 if(meta.outline&&downloaded.some(row=>meta.outline.levels.includes(row.z))){
  const records=unpack(await getBytes('/sqlite/outline?'+new URLSearchParams({tiles:downloaded.filter(row=>meta.outline.levels.includes(row.z)).map(row=>[row.z,row.x,row.y].join(',')).join(';')}),'輪郭の差分'));
  for(const row of records)outlines.set([row.z,row.x,row.y].join(','),row.body);
 }
 for(let i=0;i<downloaded.length;i++){
  const row=downloaded[i],key=[row.z,row.x,row.y].join(','),decoded=await decodeMesh(row.body);
  if(outlines.has(key)){
   const raw=await unzip(outlines.get(key),'gzip'),view=new DataView(raw.buffer,raw.byteOffset,raw.byteLength);
   if(raw.length%16)throw Error('輪郭の差分が不正です');decoded.originalPoints=decoded.points;decoded.points=decoded.points.map(p=>[...p]);
   for(let j=0;j<raw.length;j+=16){const position=view.getFloat32(j,true);if(!Number.isInteger(position)||position<0||position>=decoded.points.length)throw Error('輪郭の頂点が不正です');for(let k=0;k<3;k++)decoded.points[position][k]+=view.getFloat32(j+4+k*4,true);}
  }
  const value={...decoded,bytes:row.body.length+(outlines.get(key)?.length||0)};
  rows.set(key,cacheTile(key,value));
  progress(task,'地形',{detail:`展開 ${i+1} / ${downloaded.length} タイル`});
 }
 progress(task,'地形',{detail:'座標変換・タイル接続'});
 const result=assembleTerrain(plan,rows,meta.terrain,lat,lon,size,view,fixed);
 if(meta.outline&&meta.outline.levels.includes(result.coverage.zoom))Object.assign(result.coverage,{outlineZoom:result.coverage.zoom,outlinePrecomputed:true,outlineSmoothed:true});
 payloadCache.set(key,result);while(payloadCache.size>2)payloadCache.delete(payloadCache.keys().next().value);
 return result;
}
async function layers(base,meta,task){
 const p=structuredClone(base),bounds=requestedBounds(p),query=new URLSearchParams(Object.fromEntries(['west','south','east','north'].map((k,i)=>[k,bounds[i]]))),level=lineLevel(p.coverage.sizeKm,p.projectionCenter[1],meta.lines?.levels||[4]);
 query.set('level',level);
 const [lineBytes,waterBytes,points]=await Promise.all([
  meta.lines?.complete?getBytes('/sqlite/lines?'+query,'河川・徒歩道・県境'):Promise.resolve(null),
  meta.water&&p.coverage.sizeKm<=100?getBytes('/sqlite/water?'+query,'水域'):Promise.resolve(null),
  peaks().catch(()=>[])
 ]);
 progress(task,'追加レイヤー',{phase:'decoding',detail:'線と水域の展開'});
 const lineRows=[],waterRows=[];let sourceBytes=base.transferSize.compressed;
 for(const row of lineBytes?unpack(lineBytes):[]){lineRows.push({kind:row.kind,lines:decodeLines(await unzip(row.body,'gzip'))});sourceBytes+=row.body.length;}
 for(const row of waterBytes?unpack(waterBytes):[]){waterRows.push({kind:row.kind,item:JSON.parse(new TextDecoder().decode(await unzip(row.body,'gzip')))});sourceBytes+=row.body.length;}
 p.overlays=emptyOverlays(meta,level);
 progress(task,'追加レイヤー',{detail:'地形の標高検索を準備'});const surface=new TerrainSurface(p.mesh,p.indices);
 progress(task,'追加レイヤー',{detail:'水面・岸の三角形を作成'});appendWater(p,waterRows,surface);
 progress(task,'追加レイヤー',{detail:'河川・徒歩道・県境の標高を補間'});appendNationalLines(p,lineRows,surface);
 if(meta.water){p.overlays.bounds=meta.water.bounds;const b=meta.water.bounds;p.overlays.waterComplete=bounds[0]>=b[0]&&bounds[1]>=b[1]&&bounds[2]<=b[2]&&bounds[3]<=b[3];}
 const c=p.coverage,delta=c.sizeKm*500/R*180/PI;p.peaks=points.filter(point=>point.name&&point.elevationM!=null&&Math.abs(point.latitude-c.latitude)<=delta&&Math.abs(point.longitude-c.longitude)<=delta/Math.cos(c.latitude*PI/180));
 const arrays=Object.values(p.overlays.layers).reduce((sum,layer)=>sum+layer.lines.length+layer.triangles.length,0);
 p.transferSize={compressed:sourceBytes,decoded:p.mesh.length*12+p.indices.byteLength+(p.landIndices?.byteLength||0)+arrays*4,source:'sqlite'};
 return p;
}
// 同じ Worker 内で復元を直列化し、XZ の作業領域と生成中の地形を共有する。
let queue=Promise.resolve();
self.onmessage=event=>{const {id,url,base:providedBase}=event.data;queue=queue.then(async()=>{
 const task='sqlite-task-'+id;
 try{
  const query=new URL(url,self.location.href).searchParams,meta=await metadata();
  progress(task,query.get('stage')==='full'?'追加レイヤー':'地形',{phase:'decoding',received:0,compressed:null,detail:'使用するタイルを選択'});
  if(meta.terrain?.schemaFormat!=='yamamuki-terrain-mesh')throw Error('ポリゴン形式の地形 SQLite が必要です');
  const base=query.get('stage')==='overview'?await overview(query,meta,task):(providedBase||await terrain(query,meta,task)),payload=query.get('stage')==='full'?await layers(base,meta,task):base;
  postMessage({type:'result',id,payload});
 }catch(error){postMessage({type:'error',id,error:error.message});}
 finally{postMessage({type:'finish-progress',key:task});}
});};
