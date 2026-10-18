const assert=require('node:assert/strict'),nav=require('./national-navigation.js');
const near=(a,b)=>assert.ok(Math.abs(a-b)<1e-7,`${a} != ${b}`);
for(const bearing of [0,.7,Math.PI/2,Math.PI])for(const tilt of [0,.6,75*Math.PI/180]){
 const center=[10000,20000],scale=12000,height=900,dx=137,dy=-81;
 const next=nav.pan(center,dx,dy,scale,height,bearing,tilt),east=center[0]-next[0],north=next[1]-center[1],unit=2*scale/height;
 near((Math.cos(bearing)*east+Math.sin(bearing)*north)/unit,dx);
 near(-(-Math.sin(bearing)*east+Math.cos(bearing)*north)*Math.cos(tilt)/unit,dy);
 const before=nav.offset(dx,dy,scale,height,bearing,tilt),zoom=nav.zoom(center,dx,dy,scale,scale*.6,height,bearing,tilt),after=nav.offset(dx,dy,scale*.6,height,bearing,tilt);
 near(center[0]+before[0],zoom[0]+after[0]);near(center[1]+before[1],zoom[1]+after[1]);
}
console.log('ドラッグの画面投影一致・回転4方向・傾き3段階・ズーム中心保持 OK');
