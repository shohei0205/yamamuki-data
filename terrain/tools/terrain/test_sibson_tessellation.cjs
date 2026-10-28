// 近景への頂点配分と、共有辺を持たない斜めタイルの誤参照を確認する。
const fs=require('node:fs'),vm=require('node:vm'),assert=require('node:assert/strict');
const source=fs.readFileSync('tools/terrain/sibson_region128.js','utf8');
const context={innerWidth:1440,innerHeight:1000,yaw:-.3652,tilt:.9376,span:7549,pan:[68460.54,50453.51]};vm.createContext(context);
vm.runInContext(source.slice(source.indexOf('function tileFootprint'),source.indexOf('function plan')),context);
const tiles=[{n:1024,area:600000,pixels:1500},...Array.from({length:20},()=>({n:256,area:10000,pixels:100}))];context.tiles=tiles;vm.runInContext('allocateGeometry(tiles,1800000)',context);assert.equal(tiles[0].n,1024);assert.ok(tiles.reduce((s,t)=>s+(t.n+1)**2,0)<=1800000);assert.ok(tiles.slice(1).some(t=>t.n<=128));
for(let y=0;y<16;y++)for(let x=0;x<16;x++){const f=context.tileFootprint(x,y);assert.ok(Number.isFinite(f.pixels)&&Number.isFinite(f.area));assert.ok(f.pixels<=1440.01);assert.ok(f.area<=1440000.01);}
assert.ok(context.tileFootprint(8,6).area>context.tileFootprint(9,10).area);
context.evaluate=(_owner,x,y)=>[x*.01+y*y*.0001,.01,y*.0002];vm.runInContext(source.slice(source.indexOf('function stitched'),source.indexOf('function requestBuild')),context);
const owner={info:{bounds:[8000,8000,16000,16000],n:1024,error:25}},diagonal={info:{bounds:[0,0,8000,8000],n:16,error:25}},left={info:{bounds:[0,8000,8000,16000],n:256,error:25}};
const actual=context.stitched(owner,8000,10010,[owner,diagonal,left]),a=context.evaluate(owner,8000,10000),b=context.evaluate(owner,8000,10031.25),fraction=10/31.25;assert.ok(Math.abs(actual[0]-(a[0]*(1-fraction)+b[0]*fraction))<1e-8);
console.log('近景配分・予算・投影範囲・斜めタイルの辺整合：成功');
