import assert from 'node:assert/strict';
import {selectCoverage} from './sibson_gpu_variable_selection.mjs';
const levels=[{error:800,side:128000},{error:400,side:64000}],map=new Map(),resident=new Map();
for(let level=0;level<2;level++)for(let y=0;y<2**level;y++)for(let x=0;x<2**level;x++){const {error,side}=levels[level],t={error,side,x,y,n:128,level,bounds:[x*side,y*side,(x+1)*side,(y+1)*side]};map.set([error,x,y].join('/'),t);}
const root=map.get('800/0/0');resident.set('root',{offset:0,tile:root});const desired=[...map.values()].filter(t=>t.error===400),select=()=>selectCoverage({levels,map,desired,resident,visible:()=>true,gridN:()=>128});
let draw=select();assert.equal(draw.length,1);assert.equal(draw[0].tile.error,800);
for(let i=0;i<3;i++)resident.set('child'+i,{offset:(i+1)*1024,tile:desired[i]});
draw=select();assert.equal(draw.length,4);assert.equal(draw.filter(v=>v.tile.error===400).length,3);const fallback=draw.find(v=>v.tile.error===800);assert.deepEqual(fallback.drawBounds,desired[3].bounds);assert.equal(fallback.drawN,64);
assert.equal(draw.reduce((sum,v)=>sum+(v.drawBounds[2]-v.drawBounds[0])*(v.drawBounds[3]-v.drawBounds[1]),0),128000**2);
resident.set('child3',{offset:4096,tile:desired[3]});resident.set('coarse-cache',{offset:8192,tile:{...desired[0],n:32}});desired[0]={...desired[0],n:64};draw=select();assert.ok(draw.every(v=>v.tile.error===400));const first=draw.find(v=>v.tile.x===0&&v.tile.y===0);assert.equal(first.tile.n,128);assert.equal(first.drawN,64);
levels.push({error:200,side:32000});for(let y=0;y<4;y++)for(let x=0;x<4;x++)map.set([200,x,y].join('/'),{error:200,side:32000,x,y,n:128,level:2,bounds:[x*32000,y*32000,(x+1)*32000,(y+1)*32000]});
resident.delete('child0');resident.delete('coarse-cache');resident.set('fine',{offset:16384,tile:map.get('200/0/0')});draw=select();assert.ok(draw.some(v=>v.tile.error===200));resident.set('child0',{offset:1024,tile:desired[0]});draw=select();assert.ok(!draw.some(v=>v.tile.error===200));
console.log('部分補完・キャッシュ再利用・目標の粗いタイルが未読の間の詳細保持を確認');
const auto=selectCoverage({levels,map,desired:[root],resident,visible:()=>true,gridN:()=>128,preferCachedChild:true});assert.ok(auto.every(v=>v.tile.error===400));assert.equal(auto.length,4);assert.ok(auto.reduce((n,v)=>n+v.drawN**2*2,0)<=128**2*2);
const fixed=selectCoverage({levels,map,desired:[root],resident,visible:()=>true,gridN:()=>128});assert.equal(fixed.length,1);assert.equal(fixed[0].tile.error,800);
resident.delete('child3');const partialAuto=selectCoverage({levels,map,desired:[root],resident,visible:()=>true,gridN:()=>128,preferCachedChild:true});assert.equal(partialAuto.filter(v=>v.tile.error===400).length,3);assert.equal(partialAuto.filter(v=>v.tile.error===800).length,1);assert.ok(!partialAuto.some(v=>v.tile.error===200));
console.log('自動だけ既読の1段細レベルを優先・部分保持・2段先は使わないことを確認');

// 視錐台で兄弟が除外され、深い階層の1枚だけが可視でも祖先へ拡張しない。
const deepLevels=[800,400,200,100,50,25,12.5].map((error,i)=>({error,side:128000/2**i})),deepMap=new Map();
for(let level=0;level<deepLevels.length;level++){const {error,side}=deepLevels[level];for(let y=0;y<2**level;y++)for(let x=0;x<2**level;x++)deepMap.set([error,x,y].join('/'),{error,side,x,y,n:128,level,bounds:[x*side,y*side,(x+1)*side,(y+1)*side]});}
const leaf=deepMap.get('25/16/16'),deepRoot=deepMap.get('800/0/0'),atPoint=t=>t.bounds[0]<=64001&&t.bounds[2]>64001&&t.bounds[1]<=64001&&t.bounds[3]>64001;
const oneVisible=selectCoverage({levels:deepLevels,map:deepMap,desired:[leaf],resident:new Map([['root',{offset:0,tile:deepRoot}],['leaf',{offset:1024,tile:leaf}]]),visible:atPoint,gridN:()=>128});
assert.equal(oneVisible.length,1);assert.deepEqual(oneVisible[0].drawBounds,leaf.bounds);assert.equal(oneVisible[0].drawN,128);
for(const v of oneVisible){const a=v.tile.bounds,b=v.drawBounds;assert.ok(a[0]<=b[0]&&a[1]<=b[1]&&a[2]>=b[2]&&a[3]>=b[3]);}
console.log('可視の4kmタイル1枚を64km/128kmの祖先領域へ引き伸ばさないことを確認');

// ターゲット未取得中は、中間レベルがあっても最詳細キャッシュまでたどる。
const missingRootCache=new Map([['middle',{offset:1024,tile:map.get('400/0/0')}],['fine',{offset:2048,tile:map.get('200/0/0')}] ]);
let pendingDraw=selectCoverage({levels,map,desired:[root],resident:missingRootCache,visible:()=>true,gridN:()=>128});
assert(pendingDraw.some(v=>v.tile.error===200));assert(pendingDraw.some(v=>v.tile.error===400));
missingRootCache.set('target',{offset:4096,tile:root});
let settledDraw=selectCoverage({levels,map,desired:[root],resident:missingRootCache,visible:()=>true,gridN:()=>128});assert.equal(settledDraw.length,1);assert.equal(settledDraw[0].tile.error,800);
// 同品質でも必要な曲面キャッシュ解像度が未取得なら詳細キャッシュを維持。
missingRootCache.set('target',{offset:4096,tile:{...root,n:32}});
pendingDraw=selectCoverage({levels,map,desired:[root],resident:missingRootCache,visible:()=>true,gridN:()=>128});assert(pendingDraw.some(v=>v.tile.error===200));
console.log('目標未取得・解像度不足中は全階層の最詳細キャッシュを優先し、取得後は目標へ収束');

// 同じ128km範囲の5120→2560→1280と、その先の四分割を混在させる。
const sameLevels=[{error:5120,side:128000},{error:2560,side:128000},{error:1280,side:128000},{error:640,side:64000}],sameMap=new Map();
for(let level=0;level<sameLevels.length;level++){const {error,side}=sameLevels[level],n=128000/side;for(let y=0;y<n;y++)for(let x=0;x<n;x++)sameMap.set([error,x,y].join('/'),{error,side,x,y,n:128,level,bounds:[x*side,y*side,(x+1)*side,(y+1)*side]});}
const sameCache=new Map([...sameMap].map(([key,tile],i)=>[key,{tile,offset:i*1024}]));
for(const error of [5120,2560,1280,640]){const desired=[...sameMap.values()].filter(t=>t.error===error),result=selectCoverage({levels:sameLevels,map:sameMap,desired,resident:sameCache,visible:()=>true,gridN:()=>128});assert(result.every(v=>v.tile.error===error));assert.equal(result.length,error===640?4:1);}
console.log('同範囲の粗い3品質と四分割品質の選択を確認');
