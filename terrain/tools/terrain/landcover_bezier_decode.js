// BZF1: shared cubic/linear paths and signed references from polygon rings.
// This is rendering-time subdivision, not generation of distributed land-cover data.
export function decodeBezierCoverage(buffer,info,showControls=false) {
 const view=new DataView(buffer);let at=0;
 const u8=()=>view.getUint8(at++),u32=()=>{const v=view.getUint32(at,true);at+=4;return v;},i32=()=>{const v=view.getInt32(at,true);at+=4;return v;};
 const f64=()=>{const v=view.getFloat64(at,true);at+=8;if(!Number.isFinite(v))throw Error('BZF1 coordinate');return v;};
 const point=()=>[f64(),f64()];
 const magic=String.fromCharCode(u8(),u8(),u8(),u8());
 const compact=magic==='BZF2';
 if(!compact&&magic!=='BZF1')throw Error('Unknown curve format');
 function variable(){
  let value=0,factor=1;
  for(let i=0;i<5;i++){
   const byte=u8();value+=(byte&127)*factor;
   if(byte<128)return value;
   factor*=128;
  }
  throw Error('BZF2 integer');
 }
 const countValue=compact?variable:u32;
 const chains=countValue(),polygons=countValue();
 if(chains>5000000||polygons>5000000)throw Error('Curve count');
 const scratch=new DataView(new ArrayBuffer(8));
 function coordinateDelta(previous){
  const header=u8();
  if(!header)return previous;
  const low=header%8,count=Math.floor(header/8);
  if(!count||low+count>8)throw Error('BZF2 coordinate window');
  scratch.setFloat64(0,previous,true);
  for(let i=low;i<low+count;i++)scratch.setUint8(i,scratch.getUint8(i)^u8());
  const value=scratch.getFloat64(0,true);
  if(!Number.isFinite(value))throw Error('BZF2 coordinate');
  return value;
 }
 const deltaPoint=previous=>[coordinateDelta(previous[0]),coordinateDelta(previous[1])];
 const table=[];
 if(compact){
  const count=variable();
  if(count>5000000)throw Error('BZF2 endpoint count');
  let previous=[0,0];
  for(let i=0;i<count;i++){previous=deltaPoint(previous);table.push(previous);}
 }
 const distance=(p,a,b)=>{const dx=b[0]-a[0],dy=b[1]-a[1],den=dx*dx+dy*dy,t=den>0?Math.min(1,Math.max(0,((p[0]-a[0])*dx+(p[1]-a[1])*dy)/den)):0;const x=p[0]-(a[0]+dx*t),y=p[1]-(a[1]+dy*t);return Math.sqrt(x*x+y*y);};
 const half=(a,b)=>[(a[0]+b[0])*.5,(a[1]+b[1])*.5];
 function flatten(p,out,depth=0){
  const[a,b,c,d]=p;if(depth>=16||Math.max(distance(b,a,d),distance(c,a,d))<=info.renderFlatnessM){out.push(d);return;}
  const ab=half(a,b),bc=half(b,c),cd=half(c,d),abc=half(ab,bc),bcd=half(bc,cd),mid=half(abc,bcd);
  flatten([a,ab,abc,mid],out,depth+1);flatten([mid,bcd,cd,d],out,depth+1);
 }
 const samples=[],endpoints=[],handles=[];
 const classes=showControls?new Uint8Array(chains):null;
 const endpointOffsets=showControls?new Uint32Array(chains+1):null;
 const handleOffsets=showControls?new Uint32Array(chains+1):null;
 function record(target,p){if(showControls)target.push(p[0],p[1]);}
 for(let id=0;id<chains;id++){
  const count=countValue();
  if(!count||count>5000000)throw Error('Curve segment count');
  let start=compact?table[variable()]:point();
  const last=compact?table[variable()]:null;
  if(!start||(compact&&!last))throw Error('BZF2 endpoint reference');
  const path=[start];record(endpoints,start);
  for(let i=0;i<count;i++){
   const tag=u8();
   if(tag!==0&&tag!==1)throw Error('Curve segment tag');
   let b,c,end;
   if(compact){
    if(tag===1){b=deltaPoint(start);c=deltaPoint(start);}
    end=i+1===count?last:deltaPoint(start);
   }else{
    if(tag===1){b=point();c=point();}
    end=point();
   }
   if(tag===1){record(handles,b);record(handles,c);flatten([start,b,c,end],path);}
   else path.push(end);
   record(endpoints,end);start=end;
  }
  samples.push(path);
  if(showControls){
   endpointOffsets[id+1]=endpoints.length;
   handleOffsets[id+1]=handles.length;
  }
 }
 const coords=Array.from({length:5},()=>[]);
 for(let id=0;id<polygons;id++){
  const code=u8(),count=countValue();if(code>4||!count||count>1000000)throw Error('BZF1 polygon');
  const rings=[];
  for(let r=0;r<count;r++){
   const n=countValue();if(!n||n>1000000)throw Error('BZF1 ring');const ring=[];
   for(let j=0;j<n;j++){
    const encoded=compact?variable():0;
    const ref=compact?(Math.floor(encoded/2)+1)*(encoded%2?-1:1):i32(),path=samples[Math.abs(ref)-1];if(!ref||!path)throw Error('BZF1 reference');
    if(showControls)classes[Math.abs(ref)-1]|=1<<code;
    if(ref>0){for(let k=0;k<path.length-1;k++)ring.push(path[k]);}
    else {for(let k=path.length-1;k>0;k--)ring.push(path[k]);}
   }
   if(ring.length<3)throw Error('BZF1 degenerate ring');ring.push(ring[0]);rings.push(ring);
  }coords[code].push(rings);
 }
 if(at!==buffer.byteLength)throw Error('BZF1 trailing bytes');
 return {type:'FeatureCollection',curveInfo:info,controlPoints:showControls?{endpoints:new Float32Array(endpoints),handles:new Float32Array(handles),classes,endpointOffsets,handleOffsets}:undefined,features:coords.map((c,code)=>({type:'Feature',properties:{class:code},geometry:{type:'MultiPolygon',coordinates:c}}))};
}
