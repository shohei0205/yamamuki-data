const assert=require('node:assert/strict'),h=require('./height-boundaries.js');
const cache=new Map(),index=new Map();function add(z,x,y,height){const key=`${z}/${x}/${y}`;index.set(key,true);cache.set(key,{heights:new Float32Array(65536).fill(height),validValues:new Uint8Array(65536).fill(255)});}
add(12,3600,1600,100);add(12,3601,1600,300);add(12,3600,1601,500);add(12,3601,1601,700);
assert.deepEqual(h.sample(3601/4096,1600.5/4096,12,cache,index),[200,1]);assert.deepEqual(h.sample(3601/4096,1601/4096,12,cache,index),[400,1]);
const n=128,a=h.edges([12,3600,1600],[0,0,1,1],n,12,cache,index),b=h.edges([12,3601,1600],[0,0,1,1],n,12,cache,index);for(let i=0;i<=n;i++)assert.deepEqual([...a.slice(((n+1)+i)*2,((n+1)+i)*2+2)],[...b.slice((3*(n+1)+i)*2,(3*(n+1)+i)*2+2)]);
const coarse=h.edges([11,1800,800],[0,0,.5,.5],n,12,cache,index);for(let i=0;i<=n;i++)assert.deepEqual([...coarse.slice(((n+1)+i)*2,((n+1)+i)*2+2)],[...b.slice((3*(n+1)+i)*2,(3*(n+1)+i)*2+2)]);
add(11,1800,800,900);index.set('12/3601/1600',true);cache.delete('12/3601/1600');assert.deepEqual(h.sample(3601/4096,1600.5/4096,12,cache,index),[900,1]);
cache.delete('11/1800/800');assert.deepEqual(h.sample(3601/4096,1600.5/4096,12,cache,index),[100,1]);
console.log('共有境界：2タイル平均・4タイル角・全境界頂点一致・未取得時の共通親標高 OK');
