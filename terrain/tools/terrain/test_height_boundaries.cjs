const assert=require('node:assert/strict'),h=require('./height-boundaries.js');
const cache=new Map(),index=new Map();function add(z,x,y,height){const key=`${z}/${x}/${y}`;index.set(key,true);cache.set(key,{heights:new Float32Array(65536).fill(height),validValues:new Uint8Array(65536).fill(255)});}
add(12,3600,1600,100);add(12,3601,1600,300);add(12,3600,1601,500);add(12,3601,1601,700);
assert.deepEqual(h.sample(3601/4096,1600.5/4096,12,cache,index),[200,1]);assert.deepEqual(h.sample(3601/4096,1601/4096,12,cache,index),[400,1]);
const n=128,a=h.edges([12,3600,1600],[0,0,1,1],n,12,cache,index),b=h.edges([12,3601,1600],[0,0,1,1],n,12,cache,index);for(let i=0;i<=n;i++)assert.deepEqual([...a.slice(((n+1)+i)*2,((n+1)+i)*2+2)],[...b.slice((3*(n+1)+i)*2,(3*(n+1)+i)*2+2)]);
const coarse=h.edges([11,1800,800],[0,0,.5,.5],n,12,cache,index);for(let i=0;i<=n;i++)assert.deepEqual([...coarse.slice(((n+1)+i)*2,((n+1)+i)*2+2)],[...b.slice((3*(n+1)+i)*2,(3*(n+1)+i)*2+2)]);
add(11,1800,800,900);index.set('12/3601/1600',true);cache.delete('12/3601/1600');assert.deepEqual(h.sample(3601/4096,1600.5/4096,12,cache,index),[900,1]);
cache.delete('11/1800/800');assert.deepEqual(h.sample(3601/4096,1600.5/4096,12,cache,index),[100,1]);
console.log('共有境界：2タイル平均・4タイル角・全境界頂点一致・未取得時の共通親標高 OK');

const plane=new Map(),planeIndex=new Map(),pixel=40075016.68557849/(4096*256);
for(let y=1600;y<=1601;y++)for(let x=3600;x<=3601;x++){const heights=new Float32Array(65536);for(let j=0;j<256;j++)for(let i=0;i<256;i++)heights[j*256+i]=(x*256+i)*2+(y*256+j)*3;const key=`12/${x}/${y}`;planeIndex.set(key,true);plane.set(key,{heights,validValues:new Uint8Array(65536).fill(255)});}
const ga=h.gradients([12,3600,1600],[0,0,1,1],128,12,plane,planeIndex),gb=h.gradients([12,3601,1600],[0,0,1,1],128,12,plane,planeIndex);for(let i=0;i<=128;i++)assert.deepEqual([...ga.slice((129+i)*2,(129+i)*2+2)],[...gb.slice((3*129+i)*2,(3*129+i)*2+2)]);
assert.ok(Math.abs(ga[(129+64)*2]-2/pixel)<1e-6);assert.ok(Math.abs(ga[(129+64)*2+1]+3/pixel)<1e-6);console.log('境界の勾配：平面の正しい法線・隣接タイルの全頂点一致 OK');
