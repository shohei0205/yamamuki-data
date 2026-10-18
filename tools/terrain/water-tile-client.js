/* RIV1/WAT1 を表示範囲のローカル座標へ変換する。 */
'use strict';
window.WaterTileClient={
 request(frame,view,widthKm){
  const auto=widthKm<=400?12:widthKm<=1200?11:10,select=id=>{const v=document.getElementById(id)?.value;return v&&v!=='auto'?Number(v):auto;},seaZ=select('seaLevel'),riverZ=select('riverLevel'),z=Math.max(seaZ,riverZ),n=2**z;
  const left=Math.max(0,view[0]),top=Math.max(0,view[1]),right=Math.min(1,view[2]),bottom=Math.min(1,view[3]);
  if(left>=right||top>=bottom)return null;
  const x0=Math.max(0,Math.floor((frame[0]+left*frame[2])*n)-1),y0=Math.max(0,Math.floor((frame[1]+top*frame[3])*n)-1);
  const x1=Math.min(n-1,Math.floor((frame[0]+right*frame[2])*n)+1),y1=Math.min(n-1,Math.floor((frame[1]+bottom*frame[3])*n)+1);
  const bounds=[x0/n,y0/n,(x1-x0+1)/n,(y1-y0+1)/n],key=[z,seaZ,riverZ,x0,x1,y0,y1].join('-');
  // 描画範囲はタイル境界へ広げず、画面周辺だけに絞る。小さな移動では再描画しない。
  const crop=(a,b,lo,hi)=>{const span=2**(Math.ceil(Math.log2((b-a)*1.3)*4)/4),step=span/16,c=Math.round((a+b)/2/step)*step;return [Math.max(lo,c-span/2),Math.min(hi,c+span/2)];};
  const xx=crop(frame[0]+left*frame[2],frame[0]+right*frame[2],bounds[0],bounds[0]+bounds[2]),yy=crop(frame[1]+top*frame[3],frame[1]+bottom*frame[3],bounds[1],bounds[1]+bounds[3]);
  const maskBounds=[xx[0],yy[0],xx[1]-xx[0],yy[1]-yy[0]];
  return {key,z,seaZ,riverZ,x0,x1,y0,y1,bounds,maskBounds,frame:[...frame]};
 },
 decode(data,request){
  const view=new DataView(data);if(data.byteLength<8||view.getUint32(0,true)!==0x31515359)throw Error('水域タイルの応答形式が不正です');
  const length=view.getUint32(4,true);if(length>data.byteLength-8)throw Error('水域タイルの見出しが不正です');
  const rows=JSON.parse(new TextDecoder().decode(new Uint8Array(data,8,length))),groups=[[],[],[],[],[]],features=[0,0,0];let start=8+length,seaTiles=0;
  const triangulate=(rings,target)=>{const flat=[],holes=[];let points=0;for(let i=0;i<rings.length;i++){if(i)holes.push(points);for(const p of rings[i])flat.push(...p);points+=rings[i].length;}for(const i of earcut.default(flat,holes,2))target.push(flat[i*2],flat[i*2+1]);};
  for(const row of rows){
   if(!Number.isInteger(row.bytes)||row.bytes<8||start+row.bytes>data.byteLength||!['rivers','water','sea'].includes(row.layer)||row.z!==(row.layer==='sea'?(request.seaZ||request.z):(request.riverZ||request.z)))throw Error('水域タイルのサイズ・種別が不正です');
   const raw=new DataView(data,start,row.bytes),magic=raw.getUint32(0,true),count=raw.getUint32(4,true);start+=row.bytes;let offset=8;
   const uint=()=>{if(offset+4>raw.byteLength)throw Error('水域の頂点数が不足しています');const v=raw.getUint32(offset,true);offset+=4;return v;};
   const point=(x,y)=>[((row.x+x/4096)/2**row.z-request.bounds[0])/request.bounds[2],((row.y+y/4096)/2**row.z-request.bounds[1])/request.bounds[3]];
   const ring=()=>{const n=uint();if(n>(raw.byteLength-offset)/4)throw Error('水域の頂点が不足しています');const p=[];for(let i=0;i<n;i++){const x=raw.getUint16(offset,true),y=raw.getUint16(offset+2,true);offset+=4;if(x>4096||y>4096)throw Error('水域の座標が範囲外です');p.push(point(x,y));}return p;};
   if(row.layer==='rivers'){
    if(magic!==0x31564952)throw Error('河川形式が不正です');
    for(let i=0;i<count;i++){const p=ring();for(let j=1;j<p.length;j++)groups[0].push(...p[j-1],...p[j]);features[0]++;}
   }else{
    if(magic!==0x31544157)throw Error('水面形式が不正です');
    if(row.layer==='sea'){seaTiles++;const a=point(0,0),b=point(4096,4096);groups[3].push(...a,b[0],a[1],a[0],b[1],b[0],a[1],...b,a[0],b[1]);}
    for(let i=0;i<count;i++){const kind=uint(),nr=uint();if(kind>1||nr<1||nr>(raw.byteLength-offset)/4)throw Error('水面のリングが不正です');const rings=[];for(let r=0;r<nr;r++)rings.push(ring());triangulate(rings,groups[row.layer==='sea'?4:1+kind]);if(row.layer!=='sea')features[1+kind]++;}
   }
   if(offset!==raw.byteLength)throw Error('水域タイルの終端が不正です');
  }
  if(start!==data.byteLength)throw Error('水域応答の終端が不正です');
  const vertices=new Float32Array(groups.flat()),counts=groups.map(g=>g.length/2);
  return {vertices,counts,features,seaTiles,bytes:data.byteLength};
 }
};
