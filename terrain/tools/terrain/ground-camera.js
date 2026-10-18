/* 俯瞰・水平表示の画面座標を、地形の座標へ戻す。 */
'use strict';
const GroundCamera={
 unproject(nx,ny,depth,{scale,aspect,bearing,tilt,perspective=false,pitch=0,eyeHeight=0,distance=50000,fov=Math.PI/3}){
  let x,y,z;
  if(perspective){const near=.5,f=1/Math.tan(fov/2),d=depth*2-1,forward=2*distance*near/(distance+near-d*(distance-near)),vertical=ny*forward/f;x=nx*forward*aspect/f;y=forward*Math.cos(pitch)-vertical*Math.sin(pitch);z=eyeHeight+forward*Math.sin(pitch)+vertical*Math.cos(pitch);}
  else{const qy=ny*scale,qz=(.5-depth)*40000000;x=nx*scale*aspect;y=qy*Math.cos(tilt)-qz*Math.sin(tilt);z=qy*Math.sin(tilt)+qz*Math.cos(tilt);}
  return [Math.cos(bearing)*x-Math.sin(bearing)*y,Math.sin(bearing)*x+Math.cos(bearing)*y,z];
 },
 surface(values,range,n,edges,indices,u,v){
  const x=Math.max(0,Math.min(n-1,Math.floor(u*n))),y=Math.max(0,Math.min(n-1,Math.floor(v*n))),vertex=i=>{const gx=i%(n+1),gy=Math.floor(i/(n+1));let e=-1,t=0;if(gy===0){e=0;t=gx;}else if(gx===n){e=1;t=gy;}else if(gy===n){e=2;t=gx;}else if(gx===0){e=3;t=gy;}const h=e>=0?edges[(e*(n+1)+t)*2]:HeightMesh.sample(values,range[0]+gx/n*range[2],range[1]+gy/n*range[3]);return [gx/n,gy/n,h];};
  for(let t=0;t<2;t++){const offset=(y*n+x)*6+t*3,a=vertex(indices[offset]),b=vertex(indices[offset+1]),c=vertex(indices[offset+2]),den=(b[1]-c[1])*(a[0]-c[0])+(c[0]-b[0])*(a[1]-c[1]),wa=((b[1]-c[1])*(u-c[0])+(c[0]-b[0])*(v-c[1]))/den,wb=((c[1]-a[1])*(u-c[0])+(a[0]-c[0])*(v-c[1]))/den,wc=1-wa-wb;if(Math.min(wa,wb,wc)>=-1e-5)return wa*a[2]+wb*b[2]+wc*c[2];}
  return null;
 }
};
if(typeof module!=='undefined')module.exports=GroundCamera;
