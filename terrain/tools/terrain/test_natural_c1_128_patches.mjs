import {makePatches,controlDensity} from './natural_c1_128_patches.mjs';
import assert from 'node:assert/strict';
const tiles=[{bounds:[0,0,2048,2048],n:512},{bounds:[2048,0,4096,2048],n:512}];
const p=makePatches(tiles,(x,y)=>({z:200+y*20}),1400);
assert.equal(p.length,32);assert(new Set(p.map(v=>v.n)).size>1);
for(const a of p)for(const b of p){if(a===b)continue;if(a.bounds[2]===b.bounds[0]&&Math.min(a.bounds[3],b.bounds[3])>Math.max(a.bounds[1],b.bounds[1]))assert.equal(a.edges[1],b.edges[0]);if(a.bounds[3]===b.bounds[1]&&Math.min(a.bounds[2],b.bounds[2])>Math.max(a.bounds[0],b.bounds[0]))assert.equal(a.edges[3],b.edges[2]);}
const capped=makePatches(tiles,()=>({z:200}),1400,10000);assert(capped.reduce((s,v)=>s+v.n*v.n*2,0)<=10000);
console.log('画面距離によるパッチ分割・タイル内外の辺同期・描画予算を確認');

const coarse=makePatches([{bounds:[0,0,64000,64000],side:64000,n:512}],(x,y)=>({z:1000+y*2}),1400);assert.equal(coarse.length,1024);assert(new Set(coarse.map(p=>p.n)).size>=3);const culled=makePatches([{bounds:[0,0,64000,64000],side:64000,n:512}],()=>({z:10000}),1400,2000000,t=>t.bounds[0]<32000);assert.equal(culled.length,512);console.log("64kmタイル内の分割差とパッチ単位の画面外除外を確認");

const reference=makePatches(tiles,()=>({z:200}),1400,Infinity).map(p=>({...p}));let total=reference.reduce((s,p)=>s+2*p.n*p.n,0);while(total>10000){const p=reference.filter(p=>p.n>1).sort((a,b)=>a.pixels/a.n-b.pixels/b.n)[0];if(!p)break;total-=1.5*p.n*p.n;p.n/=2;}assert.deepEqual(capped.map(p=>p.n),reference.map(p=>p.n));console.log("ヒープによる予算調整が従来ソートと同じ分割を選ぶことを確認");

const bytes=new ArrayBuffer(32*4),f=new Float32Array(bytes);for(let i=0;i<4;i++){f[i*8]=-.0004+i*.0001;f[i*8+1]=-.0004;}
const density=controlDensity(bytes,0,{pointBytes:bytes.byteLength,side:32,origin:[16,16]});assert.equal(density.grid.reduce((a,b)=>a+b,0),4);
const grid=new Uint32Array(1024).fill(16),dense=makePatches([{bounds:[0,0,2048,2048],side:2048,n:512,controlDensity:{grid,side:2048,bounds:[0,0,2048,2048]}}],()=>({z:1e9}),1400,1);
assert(dense.every(p=>p.n>=p.minN&&p.minN===32));assert(dense.reduce((s,p)=>s+2*p.n*p.n,0)>1);console.log('制御点密度の下限を遠景・予算調整でも維持することを確認');

const mixed=makePatches([{bounds:[0,0,32000,32000],side:32000,n:128},{bounds:[32000,0,36000,4000],side:4000,n:128}],(x,y)=>({z:1000+y*10}),1400);
for(const a of mixed.filter(p=>p.bounds[2]===32000))for(const b of mixed.filter(p=>p.bounds[0]===32000))if(Math.min(a.bounds[3],b.bounds[3])>Math.max(a.bounds[1],b.bounds[1]))assert.equal(a.edges[1],b.edges[0]);console.log('異サイズパッチの部分接触で辺全体の刻みが一致することを確認');
