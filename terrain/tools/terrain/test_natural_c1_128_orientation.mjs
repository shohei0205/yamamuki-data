import fs from 'node:fs';
import vm from 'node:vm';
import assert from 'node:assert/strict';
const source=fs.readFileSync(new URL('./natural_c1_128.js',import.meta.url),'utf8');
const code=source.slice(source.indexOf('function project('),source.indexOf('function visible('));
const ctx={pan:[0,0],yaw:0,tilt:0,focus:0,span:10000,innerWidth:1400,innerHeight:1000,Math};vm.createContext(ctx);vm.runInContext(code,ctx);
assert(ctx.project(0,-1000,0).y>0);assert(ctx.project(0,1000,0).y<0);assert(ctx.project(1000,0,0).x>0);
// 各回転角で、移動の逆変換が画面のドラッグ方向と一致する。
for(const yaw of [0,.7,Math.PI/2,Math.PI]){ctx.yaw=yaw;ctx.pan=[0,0];const a=ctx.project(0,0,0),dx=40,dy=25,k=ctx.span/ctx.innerWidth;ctx.pan=[-(Math.cos(yaw)*dx-Math.sin(yaw)*dy)*k,-(Math.sin(yaw)*dx+Math.cos(yaw)*dy)*k];const b=ctx.project(0,0,0);assert(b.x>a.x);assert(b.y<a.y);}
const wgsl=fs.readFileSync(new URL('./natural_c1_128_render.wgsl',import.meta.url),'utf8');assert.equal(wgsl.split('camera.pose.y*p.x-camera.pose.x*p.y').length-1,3);
console.log('北が上・東が右・回転時のドラッグ・3描画方式の軸整合を確認');
