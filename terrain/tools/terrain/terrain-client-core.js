/* SQLite に保存された地形と線をブラウザーで復元する。 */
'use strict';
const R=6378137,PI=Math.PI;
function tilePosition(lon,lat,z){return [(lon+180)/360*2**z,(1-Math.asinh(Math.tan(lat*PI/180))/PI)/2*2**z];}
function lonLat(x,y,z){const n=256*2**z;return [x/n*360-180,Math.atan(Math.sinh(PI*(1-2*y/n)))*180/PI];}
function meshGeometry(leaves,n=256){
 const horizontal=new Map(),vertical=new Map();
 const add=(map,k,v)=>{if(!map.has(k))map.set(k,new Set());map.get(k).add(v);};
 for(const [x,y,size] of leaves)for(const [xx,yy] of [[x,y],[x+size,y],[x+size,y+size],[x,y+size]]){add(horizontal,yy,xx);add(vertical,xx,yy);}
 for(let p=0;p<=n;p++){add(horizontal,0,p);add(horizontal,n,p);add(vertical,0,p);add(vertical,n,p);}
 for(const [k,v] of horizontal)horizontal.set(k,[...v].sort((a,b)=>a-b));
 for(const [k,v] of vertical)vertical.set(k,[...v].sort((a,b)=>a-b));
 const xy=[],indices=[],lookup=new Map();
 const vertex=(x,y)=>{const key=x+','+y;if(!lookup.has(key)){lookup.set(key,xy.length);xy.push([x,y]);}return lookup.get(key);};
 for(const [x,y,size] of leaves){
  if(size===1){const [a,b,c,d]=[[x,y],[x+1,y],[x+1,y+1],[x,y+1]].map(p=>vertex(...p));indices.push(a,b,d,b,c,d);continue;}
  const ring=[...horizontal.get(y).filter(xx=>xx>=x&&xx<x+size).map(xx=>[xx,y]),...vertical.get(x+size).filter(yy=>yy>=y&&yy<y+size).map(yy=>[x+size,yy]),...horizontal.get(y+size).filter(xx=>xx>x&&xx<=x+size).reverse().map(xx=>[xx,y+size]),...vertical.get(x).filter(yy=>yy>y&&yy<=y+size).reverse().map(yy=>[x,yy])];
  const center=vertex(x+size/2,y+size/2);
  for(let i=0;i<ring.length;i++)indices.push(center,vertex(...ring[i]),vertex(...ring[(i+1)%ring.length]));
 }
 return {xy,indices};
}
async function unzip(bytes,format){
 const stream=new Blob([bytes]).stream();
 const decoded=format==='xz'?new self['xz-decompress'].XzReadableStream(stream):stream.pipeThrough(new DecompressionStream('gzip'));
 return new Uint8Array(await new Response(decoded).arrayBuffer());
}
async function decodeMesh(body){
 if(body.byteLength<19)throw Error('地形ヘッダーが不足しています');
 const view=new DataView(body.buffer,body.byteOffset,body.byteLength),magic=new TextDecoder().decode(body.subarray(0,4));
 const size=view.getUint16(4,true),step=view.getFloat32(6,true),nv=view.getUint32(10,true),nt=view.getUint32(14,true),wide=view.getUint8(18);
 if(size!==257||nv>1000000||nt>3000000||!Number.isFinite(step)||step<=0)throw Error('地形ヘッダーが不正です');
 if(!['YMP1','YMP2','YMP4'].includes(magic))throw Error('未対応の地形形式です：'+magic);
 const raw=await unzip(body.subarray(19),'xz'),r=new DataView(raw.buffer,raw.byteOffset,raw.byteLength);let points=[],indices=[];
 if(magic==='YMP2'){
  if(wide!==0||nv>100000||nt>100000)throw Error('分割情報が不正です');
  const length=Math.ceil(nt/4);if(raw.length!==length+nv*2)throw Error('分割データの長さが不正です');
  let cursor=0;const leaves=[];
  function visit(x,y,n){if(cursor>=nt)throw Error('分割情報が不足しています');const code=(raw[Math.floor(cursor/4)]>>((cursor++%4)*2))&3;if(!code)return;if(code===1){leaves.push([x,y,n]);return;}if(code!==2||n<2)throw Error('分割情報が不正です');const half=n/2;for(const [dx,dy] of [[0,0],[half,0],[0,half],[half,half]])visit(x+dx,y+dy,half);}
  visit(0,0,256);if(cursor!==nt)throw Error('分割情報が余っています');
  const geometry=meshGeometry(leaves);if(geometry.xy.length!==nv)throw Error('頂点数が一致しません');
  let height=0;points=geometry.xy.map(([x,y],i)=>{height+=r.getInt16(length+i*2,true);return [x,y,height*step];});indices=geometry.indices;
 }else if(magic==='YMP4'){
  if(wide!==0||raw.length!==nv*12+nt*12)throw Error('海岸ポリゴンの長さが不正です');
  points=Array.from({length:nv},(_,i)=>[r.getFloat32(i*12,true),r.getFloat32(i*12+4,true),r.getFloat32(i*12+8,true)*step]);
  let index=0;indices=Array.from({length:nt*3},(_,i)=>index+=r.getInt32(nv*12+i*4,true));
 }else{
  if(wide>1||raw.length!==nv*6+nt*3*(wide?4:2))throw Error('地形データの長さが不正です');
  points=Array.from({length:nv},(_,i)=>[r.getInt16(i*6,true),r.getInt16(i*6+2,true),r.getInt16(i*6+4,true)*step]);
  indices=Array.from({length:nt*3},(_,i)=>wide?r.getUint32(nv*6+i*4,true):r.getUint16(nv*6+i*2,true));
 }
 if(points.some(p=>p.some(n=>!Number.isFinite(n)))||indices.some(i=>i<0||i>=nv))throw Error('地形の頂点が不正です');
 return {points,indices};
}
function geometryZoom(level,meta){return meta.geometryZoomByLevel?.[level]??level;}
function selection(lat,lon,size,requested,view,fixed,meta){
 if(!Number.isFinite(lat)||!Number.isFinite(lon)||!Number.isFinite(size)||lat< -80||lat>80||lon< -179||lon>179||size<2||size>4000)throw Error('表示範囲が不正です');
 const levels=[...meta.levels].sort((a,b)=>a-b),delta=size*500/R*180/PI,dlon=delta/Math.cos(lat*PI/180);
 const corners=z=>{const a=tilePosition(Math.max(-179.999,lon-dlon),Math.min(85,lat+delta),geometryZoom(z,meta)),b=tilePosition(Math.min(179.999,lon+dlon),Math.max(-85,lat-delta),geometryZoom(z,meta));return [...a,...b];};
 let budget=levels[0];
 for(let z=14;z>=4;z--){const d=size*250/R*180/PI,a=tilePosition(Math.max(-179.999,lon-d/Math.cos(lat*PI/180)),Math.min(85,lat+d),z),b=tilePosition(Math.min(179.999,lon+d/Math.cos(lat*PI/180)),Math.max(-85,lat-d),z);if((b[0]-a[0])*256*(b[1]-a[1])*256<=800000||z===4){budget=z;break;}}
 const desired=fixed?requested:Math.min(requested??budget,budget);
 if(fixed&&!levels.includes(requested))throw Error('地形レベルが不正です');
 const zoom=levels.filter(z=>z<=desired).at(-1)??levels[0];
 const [x0,y0,x1,y1]=corners(zoom).map(Math.floor);
 if((x1-x0+1)*(y1-y0+1)>256)throw Error('このレベルでは表示範囲が広すぎます。拡大してから選んでください。');
 const spatialZoom=geometryZoom(zoom,meta),offset=meta.pixelOrigin==='highest-grid-corners'?0:(1-2**(zoom-14))/2,center=lonLat((x0+x1+1)*128+offset,(y0+y1+1)*128+offset,spatialZoom);
 let tiles=[];
 for(let y=y0;y<=y1;y++)for(let x=x0;x<=x1;x++)tiles.push([zoom,x,y]);
 if(view&&view[0]>=45&&!fixed){
  const [tilt,bearing,distance]=view,angle=bearing*PI/180,inclination=Math.sin(tilt*PI/180),sx=PI/180*R*Math.cos(lat*PI/180),sy=PI/180*R;
  function refine(z,x,y){
   const scale=2**(geometryZoom(zoom,meta)-geometryZoom(z,meta)),left=Math.max(x*scale,x0),right=Math.min((x+1)*scale,x1+1),top=Math.max(y*scale,y0),bottom=Math.min((y+1)*scale,y1+1);
   if(left>=right||top>=bottom)return [];
   let depth=Infinity;
   for(const px of [left,right])for(const py of [top,bottom]){const [lo,la]=lonLat(px*256,py*256,spatialZoom);depth=Math.min(depth,(Math.sin(angle)*(lo-lon)*sx+Math.cos(angle)*(la-lat)*sy)*inclination);}
   const ce=-distance*inclination*Math.sin(angle),cn=-distance*inclination*Math.cos(angle),[lo0,la0]=lonLat(left*256,bottom*256,spatialZoom),[lo1,la1]=lonLat(right*256,top*256,spatialZoom);
   const west=(lo0-lon)*sx,east=(lo1-lon)*sx,south=(la0-lat)*sy,north=(la1-lat)*sy;
   const near=Math.hypot(ce-Math.min(east,Math.max(west,ce)),cn-Math.min(north,Math.max(south,cn))),ratio=Math.max(.15,(distance+depth)/distance);
   let target=levels[Math.max(0,levels.indexOf(zoom)-(ratio>3.5?2:ratio>1.4?1:0))];
   if(near<Math.min(4000,size*100))target=levels[Math.min(levels.length-1,levels.indexOf(zoom)+1)];
   const child=levels.find(level=>level>z);
   if(child!==undefined&&z<target){const factor=2**(geometryZoom(child,meta)-geometryZoom(z,meta)),out=[];for(let cy=y*factor;cy<(y+1)*factor;cy++)for(let cx=x*factor;cx<(x+1)*factor;cx++)out.push(...refine(child,cx,cy));return out;}
   return [[z,x,y]];
  }
  tiles=[];const base=levels[Math.max(0,levels.indexOf(zoom)-2)],factor=2**(geometryZoom(zoom,meta)-geometryZoom(base,meta));
  for(let y=Math.floor(y0/factor);y<=Math.floor(y1/factor);y++)for(let x=Math.floor(x0/factor);x<=Math.floor(x1/factor);x++)tiles.push(...refine(base,x,y));
 }
 if(tiles.length>256)throw Error('視点に必要なタイルが多すぎます。範囲を狭くしてください。');
 return {tiles,zoom,center,x0,y0,x1,y1,levels};
}
function median(values){values.sort((a,b)=>a-b);const i=Math.floor(values.length/2);return values.length%2?values[i]:(values[i-1]+values[i])/2;}
function assembleTerrain(plan,rows,meta,lat,lon,size,view,fixed){
 const {tiles,zoom,center,levels}=plan,mixed=new Set(tiles.map(t=>t[0])).size>1;
 const mesh=[],indices=[],lookup=new Map(),counts={},tileIDs=[];let missing=0,bytes=0;
 for(const [z,x,y] of tiles){
  const tile=rows.get([z,x,y].join(','));if(!tile){missing++;continue;}
  tileIDs.push([z,x,y]);
  const mapping=[],spatialZoom=geometryZoom(z,meta),offset=(mixed||meta.pixelOrigin==='highest-grid-corners')?0:(1-2**(z-14))/2;bytes+=tile.bytes;
  for(let pointIndex=0;pointIndex<tile.points.length;pointIndex++){
   const [vx,vy,height]=tile.points[pointIndex],[ox,oy]=(tile.originalPoints||tile.points)[pointIndex];
   const key=[z,Math.round((x*256+ox)*1e5),Math.round((y*256+oy)*1e5)].join(',');
   if(!lookup.has(key)){lookup.set(key,mesh.length);const [lo,la]=lonLat(x*256+vx+offset,y*256+vy+offset,spatialZoom);mesh.push([(lo-center[0])*PI/180*R*Math.cos(center[1]*PI/180),(la-center[1])*PI/180*R,height]);}
   else if(Math.abs(mesh[lookup.get(key)][2]-height)>.001)throw Error('タイル境界の標高が一致しません');
   mapping.push(lookup.get(key));
  }
  for(const i of tile.indices)indices.push(mapping[i]);
  if(mixed){
   const edges=new Map();
   for(let k=0;k<tile.indices.length;k+=3){const f=tile.indices.slice(k,k+3);for(const [i,j] of [[f[0],f[1]],[f[1],f[2]],[f[2],f[0]]]){const key=Math.min(i,j)+','+Math.max(i,j);edges.set(key,(edges.get(key)||0)+1);}}
   const previous=levels.filter(l=>l<z).at(-1)??levels[0],drop=Math.max(50,meta.meshErrorBounds[previous]*2);
   for(const [edge,count] of edges){if(count!==1)continue;const [i,j]=edge.split(',').map(Number),a=tile.points[i],b=tile.points[j];if(!((a[0]===b[0]&&[0,256].includes(a[0]))||(a[1]===b[1]&&[0,256].includes(a[1]))))continue;
    const ia=mapping[i],ib=mapping[j],c=mesh.length;mesh.push([mesh[ia][0],mesh[ia][1],mesh[ia][2]-drop],[mesh[ib][0],mesh[ib][1],mesh[ib][2]-drop]);indices.push(ia,ib,c,ib,c+1,c);
   }
  }
  counts[z]=(counts[z]||0)+1;
 }
 if(!indices.length)throw Error('この範囲には表示できるポリゴンがありません');
 const alt=median(mesh.map(p=>p[2]));for(const p of mesh)p[2]-=alt;
 const meters=2*PI*R*Math.cos(center[1]*PI/180)/(256*2**geometryZoom(zoom,meta));
 const coverage={savedTiles:tiles.length-missing,missingTiles:missing,latitude:lat,longitude:lon,sizeKm:size,validSamples:mesh.length,totalSamples:mesh.length,heightQuantizationMeters:5,zoom,levels:counts,tileIDs,geometryZoomByLevel:meta.geometryZoomByLevel,pixelOrigin:meta.pixelOrigin,viewLOD:view&&!fixed?view:null,overview:zoom<14,polygon:true,meshErrorMeters:meta.meshErrorBounds[zoom],levelMode:fixed?'manual':'auto'};
 if(meta.coastlineToleranceMeters?.[zoom]!==undefined)Object.assign(coverage,{outlineZoom:zoom,outlineVector:true,outlineToleranceMeters:meta.coastlineToleranceMeters[zoom]});
 return {polygon:true,mesh,indices:new Uint32Array(indices),meshStepMeters:meters,methods:{},projectionCenter:[...center,alt],span:Math.max(plan.x1-plan.x0+1,plan.y1-plan.y0+1)*256*meters,coverage,dynamicLOD:true,transferSize:{compressed:bytes,decoded:mesh.length*12+indices.length*4,source:'sqlite'}};
}
class TerrainSurface{
 constructor(mesh,indices){
  this.mesh=mesh;this.indices=indices;this.bounds=[Infinity,Infinity,-Infinity,-Infinity];
  for(const p of mesh){this.bounds[0]=Math.min(this.bounds[0],p[0]);this.bounds[1]=Math.min(this.bounds[1],p[1]);this.bounds[2]=Math.max(this.bounds[2],p[0]);this.bounds[3]=Math.max(this.bounds[3],p[1]);}
  this.n=Math.max(8,Math.min(192,Math.ceil(Math.sqrt(indices.length/9))));this.dx=Math.max(1,(this.bounds[2]-this.bounds[0])/this.n);this.dy=Math.max(1,(this.bounds[3]-this.bounds[1])/this.n);this.cells=new Map();
  for(let i=0;i<indices.length;i+=3){const a=mesh[indices[i]],b=mesh[indices[i+1]],c=mesh[indices[i+2]];
   if(Math.abs((b[1]-c[1])*(a[0]-c[0])+(c[0]-b[0])*(a[1]-c[1]))<1e-10)continue;
   const low=this.cell(Math.min(a[0],b[0],c[0]),Math.min(a[1],b[1],c[1])),high=this.cell(Math.max(a[0],b[0],c[0]),Math.max(a[1],b[1],c[1]));
   for(let y=low[1];y<=high[1];y++)for(let x=low[0];x<=high[0];x++){const key=y*this.n+x;if(!this.cells.has(key))this.cells.set(key,[]);this.cells.get(key).push(i);}
  }
 }
 cell(x,y){return [Math.max(0,Math.min(this.n-1,Math.floor((x-this.bounds[0])/this.dx))),Math.max(0,Math.min(this.n-1,Math.floor((y-this.bounds[1])/this.dy)))];}
 height(x,y){
  if(x<this.bounds[0]||x>this.bounds[2]||y<this.bounds[1]||y>this.bounds[3])return NaN;
  const [cx,cy]=this.cell(x,y);
  for(const i of this.cells.get(cy*this.n+cx)||[]){const a=this.mesh[this.indices[i]],b=this.mesh[this.indices[i+1]],c=this.mesh[this.indices[i+2]],den=(b[1]-c[1])*(a[0]-c[0])+(c[0]-b[0])*(a[1]-c[1]);
   const w=((b[1]-c[1])*(x-c[0])+(c[0]-b[0])*(y-c[1]))/den,v=((c[1]-a[1])*(x-c[0])+(a[0]-c[0])*(y-c[1]))/den;
   if(w>=-1e-8&&v>=-1e-8&&w+v<=1+1e-8)return w*a[2]+v*b[2]+(1-w-v)*c[2];
  }return NaN;
 }
}
function clipSegment(a,b,box){
 let t0=0,t1=1;const dx=b[0]-a[0],dy=b[1]-a[1];
 for(const [p,q] of [[-dx,a[0]-box[0]],[dx,box[2]-a[0]],[-dy,a[1]-box[1]],[dy,box[3]-a[1]]]){
  if(p===0){if(q<0)return null;continue;}const t=q/p;if(p<0)t0=Math.max(t0,t);else t1=Math.min(t1,t);if(t0>t1)return null;
 }return [[a[0]+dx*t0,a[1]+dy*t0],[a[0]+dx*t1,a[1]+dy*t1]];
}
function decodeLines(raw){
 if(raw.length<8||new TextDecoder().decode(raw.subarray(0,4))!=='YVL1')throw Error('線データの形式が不正です');
 const view=new DataView(raw.buffer,raw.byteOffset,raw.byteLength),count=view.getUint32(4,true);if(count>10000000)throw Error('線の本数が不正です');
 let cursor=8,px=0,py=0;const result=[];
 function read(){let value=0,multiplier=1;for(let i=0;i<8;i++){if(cursor>=raw.length)throw Error('線データが不足しています');const b=raw[cursor++];value+=(b&127)*multiplier;if(!Number.isSafeInteger(value))throw Error('線の整数が大きすぎます');if(!(b&128))return value;multiplier*=128;}throw Error('線の整数が不正です');}
 const signed=n=>n%2?-(n+1)/2:n/2;
 for(let k=0;k<count;k++){const size=read();if(size<2||size>raw.length/2)throw Error('線の頂点数が不正です');const line=[];
  for(let i=0;i<size;i++){px+=signed(read());py+=signed(read());if(Math.abs(px)>2*PI*R||Math.abs(py)>2*PI*R)throw Error('線の座標が不正です');line.push([px,py]);}result.push(line);
 }if(cursor!==raw.length)throw Error('線データが余っています');return result;
}
function requestedBounds(p){const c=p.coverage,delta=c.sizeKm*500/R*180/PI,dlon=delta/Math.cos(c.latitude*PI/180);return [Math.max(-179.999,c.longitude-dlon),Math.max(-85,c.latitude-delta),Math.min(179.999,c.longitude+dlon),Math.min(85,c.latitude+delta)];}
function lineLevel(size,lat,levels){const desired=Math.floor(Math.log2(40075016.686*Math.cos(lat*PI/180)*1800/(Math.max(2,size)*1000*256)));return [...levels].sort((a,b)=>b-a).find(z=>z<=desired)??4;}
function emptyOverlays(meta,level){return {layers:Object.fromEntries(['water','trail','boundary'].map(k=>[k,{triangles:[],lines:[]}])),counts:{water:0,river:0,trail:0,boundary:0},estimatedLakeLevels:0,attribution:meta.lines?.attribution||meta.water?.attribution||'地理院タイル',riverLevel:level,lineLevel:level,lineLevels:meta.lines?.levels||[4,5,6,7,8,9,10,11,12,13,14],nationalLines:!!meta.lines?.complete,available:false,complete:false,waterComplete:false,lineComplete:!!meta.lines?.complete};}
function appendNationalLines(p,rows,surface){
 const [lon,lat,alt]=p.projectionCenter,sx=PI/180*R*Math.cos(lat*PI/180),sy=PI/180*R,bounds=requestedBounds(p),clip=[(bounds[0]-lon)*sx,(bounds[1]-lat)*sy,(bounds[2]-lon)*sx,(bounds[3]-lat)*sy],spacing=Math.max(15,p.coverage.sizeKm*1000/900);
 for(const {kind,lines} of rows){let target=p.overlays.layers[kind==='river'?'water':kind].lines;
  for(const line of lines){
   // 大きな保存区画のうち、表示範囲に重なる線だけ標高を補間する。
   let west=Infinity,east=-Infinity,south=Infinity,north=-Infinity;
   for(const [x,y] of line){west=Math.min(west,x);east=Math.max(east,x);south=Math.min(south,y);north=Math.max(north,y);}
   const mb=[R*bounds[0]*PI/180,R*Math.log(Math.tan(PI/4+bounds[1]*PI/360)),R*bounds[2]*PI/180,R*Math.log(Math.tan(PI/4+bounds[3]*PI/360))];
   if(east<mb[0]||west>mb[2]||north<mb[1]||south>mb[3])continue;
   const projected=line.map(([x,y])=>[(x/R*180/PI-lon)*sx,(Math.atan(Math.sinh(y/R))*180/PI-lat)*sy]);let visible=false,distance=0;
   for(let i=1;i<projected.length;i++){
    const clipped=clipSegment(projected[i-1],projected[i],clip);if(!clipped)continue;visible=true;
    const [a,b]=clipped,length=Math.hypot(b[0]-a[0],b[1]-a[1]),steps=Math.max(1,Math.ceil(length/spacing)),dash=Math.max(kind==='trail'?35:150,spacing*3);
    let previous=null;
    for(let j=0;j<=steps;j++){const t=j/steps,x=a[0]+(b[0]-a[0])*t,y=a[1]+(b[1]-a[1])*t,z=surface.height(x,y)+1.5,current=[x,y,z];
     if(previous&&Number.isFinite(z)&&Number.isFinite(previous[2])&&(kind==='river'||Math.floor((distance+(j-.5)/steps*length)/dash)%2===0))target.push(...previous,...current);
     previous=current;
    }distance+=length;
   }if(visible)p.overlays.counts[kind]++;
  }
 }p.overlays.available=rows.length>0||p.overlays.available;
}
function triangulate(polygon){const flat=earcut.flatten(polygon),triangles=earcut.default?earcut.default(flat.vertices,flat.holes,2):earcut(flat.vertices,flat.holes,2);return {flat:flat.vertices,triangles};}
function polygonBounds(polygon){const box=[Infinity,Infinity,-Infinity,-Infinity];for(const ring of polygon)for(const [x,y] of ring){box[0]=Math.min(box[0],x);box[1]=Math.min(box[1],y);box[2]=Math.max(box[2],x);box[3]=Math.max(box[3],y);}return box;}
function boxesOverlap(a,b){return a[0]<=b[2]&&a[2]>=b[0]&&a[1]<=b[3]&&a[3]>=b[1];}
function shorelinePolygons(lines){
 // タイル境界で分割された水涯線を 1 m の共通座標でつなぎ、閉じた輪を取り出す。
 const edges=[],adj=new Map(),key=p=>Math.round(p[0])+','+Math.round(p[1]);
 for(const line of lines){if(line.length<2)continue;const a=key(line[0]),b=key(line.at(-1)),i=edges.length;edges.push({line,a,b});for(const k of [a,b]){if(!adj.has(k))adj.set(k,[]);adj.get(k).push(i);}}
 const used=new Set(),polygons=[];
 for(let i=0;i<edges.length;i++){if(used.has(i))continue;let edge=edges[i],ring=[...edge.line],end=edge.b,start=edge.a;used.add(i);
  for(let guard=0;end!==start&&guard<edges.length;guard++){const next=(adj.get(end)||[]).find(index=>!used.has(index));if(next===undefined)break;edge=edges[next];used.add(next);const forward=edge.a===end;ring.push(...(forward?edge.line:[...edge.line].reverse()).slice(1));end=forward?edge.b:edge.a;}
  if(end===start&&ring.length>=4){ring[ring.length-1]=[...ring[0]];polygons.push([ring]);}
 }return polygons;
}
function appendWater(p,rows,surface){
 if(!rows.length)return;
 const [lon,lat,alt]=p.projectionCenter,sx=PI/180*R*Math.cos(lat*PI/180),sy=PI/180*R,box=requestedBounds(p),clip=[[[[(box[0]-lon)*sx,(box[1]-lat)*sy],[(box[2]-lon)*sx,(box[1]-lat)*sy],[(box[2]-lon)*sx,(box[3]-lat)*sy],[(box[0]-lon)*sx,(box[3]-lat)*sy],[(box[0]-lon)*sx,(box[1]-lat)*sy]]]];
 const groups={sea:[],lake:[],other:[]},shores=[];
 const project=ring=>ring.map(([x,y])=>[(x-lon)*sx,(y-lat)*sy]);
 for(const {kind,item} of rows){const geometry=item.geometry,code=Number(item.properties?.vt_code);
  if(kind==='lakeedge'){for(const line of geometry.type==='LineString'?[geometry.coordinates]:geometry.type==='MultiLineString'?geometry.coordinates:[])shores.push(project(line));continue;}
  const polygons=geometry.type==='Polygon'?[geometry.coordinates]:geometry.type==='MultiPolygon'?geometry.coordinates:[];
  for(const polygon of polygons){const result=polygonClipping.intersection([polygon.map(project)],clip);if(!result.length)continue;p.overlays.counts.water++;const key=code>=5100&&code<5200?'sea':code>=5230&&code<=5242?'lake':'other';groups[key].push(...result);}
 }
 let unknown=groups.other.length?polygonClipping.union(...groups.other):[];
 for(const lake of shorelinePolygons(shores)){
  const closed=unknown.length?polygonClipping.intersection([lake],unknown):[];
  if(closed.length){groups.lake.push(...closed);unknown=polygonClipping.difference(unknown,closed);}
 }
 groups.other=unknown;
 const flat=[];
 for(const [kind,polygons] of Object.entries(groups)){
  if(!polygons.length)continue;
  for(const polygon of polygonClipping.union(...polygons)){
   let height=null;
   if(kind==='sea')height=-alt;
   if(kind==='lake'){
    const heights=[];for(const ring of polygon)for(let i=1;i<ring.length;i++){const a=ring[i-1],b=ring[i],count=Math.max(1,Math.ceil(Math.hypot(b[0]-a[0],b[1]-a[1])/50));for(let j=0;j<count;j++){const t=j/count,h=surface.height(a[0]+(b[0]-a[0])*t,a[1]+(b[1]-a[1])*t);if(Number.isFinite(h))heights.push(h);}}
    if(!heights.length)continue;height=median(heights);p.overlays.estimatedLakeLevels++;
   }
   if(height!==null)flat.push({polygon,height,box:polygonBounds(polygon)});
   const {flat:xy,triangles}=triangulate(polygon);
   for(let i=0;i<triangles.length;i+=3){const vertices=triangles.slice(i,i+3).map(index=>[xy[index*2],xy[index*2+1],(height??surface.height(xy[index*2],xy[index*2+1]))+.5]);if(vertices.every(v=>Number.isFinite(v[2])))for(const v of vertices)p.overlays.layers.water.triangles.push(...v);}
  }
 }
 if(flat.length){
  const wet=polygonClipping.union(...flat.map(f=>f.polygon)),wetBox=polygonBounds(wet.flat()),land=[];
  for(let i=0;i<p.indices.length;i+=3){const ids=Array.from(p.indices.slice(i,i+3)),face=ids.map(index=>p.mesh[index].slice(0,2)),faceBox=polygonBounds([face]);
   if(!boxesOverlap(faceBox,wetBox)||!flat.some(f=>boxesOverlap(faceBox,f.box))){land.push(...ids);continue;}
   const area=(face[1][0]-face[0][0])*(face[2][1]-face[0][1])-(face[2][0]-face[0][0])*(face[1][1]-face[0][1]);if(Math.abs(area)<1e-10){land.push(...ids);continue;}
   const remaining=polygonClipping.difference([[...face,face[0]]],wet);
   for(const polygon of remaining){const {flat:xy,triangles}=triangulate(polygon);
    for(let j=0;j<triangles.length;j+=3){const vertices=triangles.slice(j,j+3).map(index=>{const x=xy[index*2],y=xy[index*2+1];let h=surface.height(x,y);for(const f of flat){if(x<f.box[0]-.02||x>f.box[2]+.02||y<f.box[1]-.02||y>f.box[3]+.02)continue;let edge=false;for(const ring of f.polygon)for(let k=1;k<ring.length;k++){const a=ring[k-1],b=ring[k],dx=b[0]-a[0],dy=b[1]-a[1],t=Math.max(0,Math.min(1,((x-a[0])*dx+(y-a[1])*dy)/(dx*dx+dy*dy||1)));if(Math.hypot(x-a[0]-t*dx,y-a[1]-t*dy)<.02){edge=true;break;}}if(edge){h=f.height;break;}}return [x,y,h];});if(vertices.every(v=>Number.isFinite(v[2]))){const start=p.mesh.length;p.mesh.push(...vertices);land.push(start,start+1,start+2);}}
   }
  }p.landIndices=new Uint32Array(land);
 }
 p.overlays.available=true;
}
if(typeof module!=='undefined')module.exports={geometryZoom,meshGeometry,decodeMesh,selection,assembleTerrain,TerrainSurface,clipSegment,decodeLines,requestedBounds,lineLevel,appendNationalLines,appendWater,shorelinePolygons};
