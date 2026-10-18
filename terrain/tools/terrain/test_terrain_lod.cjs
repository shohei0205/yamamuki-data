const assert=require('node:assert/strict'),LOD=require('./terrain-lod.js');global.TerrainLOD=LOD;const Edges=require('./height-lod-edges.js');
const roots=[[7,64,64]],index=new Map();function tree(row){index.set(row.join('/'),row);const[z,x,y]=row;if(z===14)return;for(let dy=0;dy<2;dy++)for(let dx=0;dx<2;dx++)tree([z+1,x*2+dx,y*2+dy]);}tree(roots[0]);
const p={base:7,target:14,ground:true,scale:10000,center:[64.5/128*LOD.world,64.98/128*LOD.world],bearing:0,tilt:0,height:900,fov:Math.PI/3};const visible=row=>LOD.distance(row,p)<50000;const plan=LOD.choose(roots,index,visible,p);assert.ok(plan.leaves.length<=64);const near=plan.leaves.filter(n=>LOD.distance(n.row,p)<5000).map(n=>n.row[0]),far=plan.leaves.filter(n=>LOD.distance(n.row,p)>30000).map(n=>n.row[0]);assert.ok(near.length&&far.length);assert.ok(Math.max(...near)>Math.max(...far));assert.ok(plan.requests.length<=96);assert.ok(plan.requests.every(r=>r[0]<=14));
assert.equal(LOD.limit([12,2064,2064],{...p,distanceLOD:false}),14);const crowded=LOD.choose(Array.from({length:200},(_,i)=>[13,i,1]),new Map(),()=>true,p);assert.ok(crowded.coverageLimited);assert.equal(crowded.leaves.length,64);assert.ok(crowded.requests.length<=96);
const top={...p,ground:false,target:12,tilt:0};assert.equal(LOD.limit([10,515,518],top),12);const tilted={...top,tilt:75*Math.PI/180};const north=[12,2064,2064],south=[12,2064,2078];assert.ok(LOD.limit(north,tilted)<=LOD.limit(south,tilted));
// 粗い辺の中間に細かいタイルが接続しても、同じ折れ線になる。
const coarse={row:[8,128,128],displayRow:[8,128,128],range:[0,0,1,1],lodN:16},fine={row:[9,258,256],displayRow:[9,258,256],range:[0,0,1,1],lodN:32};const sample=(u,v,z)=>[Math.sin(v*10000)*100+z*10,1],stitch=Edges.create([coarse,fine],{base:7,target:14},sample);const ce=new Float32Array(4*17*2),cg=new Float32Array(ce.length),fe=new Float32Array(4*33*2),fg=new Float32Array(fe.length);stitch(coarse,16,ce,cg);stitch(fine,32,fe,fg);for(let i=0;i<=32;i++){const t=i/4,cell=Math.min(15,Math.floor(t)),f=t-cell,a=ce[(17+cell)*2],b=ce[(17+cell+1)*2];assert.ok(Math.abs(fe[(3*33+i)*2]-(a*(1-f)+b*f))<.001);}
console.log('距離別LOD：近景詳細・遠景削減・16タイル予算・上限・混在境界の連続 OK',plan.leaves.length,Math.max(...near),Math.max(...far));

// 負荷は取得するデータレベルとタイル数で制御し、格子は常に256分割。
assert.ok(plan.leaves.length<=plan.tileBudget);
assert.ok(plan.leaves.every(n=>LOD.mesh({row:n.row},p)===256));
assert.equal(LOD.pointBudget({...p,tileBudget:plan.tileBudget}),plan.tileBudget*257**2);
const uniform=LOD.choose(roots,index,()=>true,{...p,ground:false,tilt:0});
assert.equal(new Set(uniform.leaves.map(n=>n.row[0])).size,1);
assert.ok(uniform.leaves.length<=uniform.tileBudget);
assert.ok(uniform.requests.length<=96);
const crowdedBase=LOD.choose(Array.from({length:30},(_,i)=>[7,i,1]),new Map(),()=>true,{...p,target:7});
assert.equal(crowdedBase.leaves.length,30,'最低レベルの表示範囲を予算のため欠かさない');
assert.equal(crowdedBase.tileBudget,30);assert.equal(crowdedBase.coverageLimited,false);
console.log('自動：16タイル予算・最低レベルの範囲維持・真上の一律レベル・256分割固定 OK');

// 手動では自動予算を超えても選んだデータレベルと分割数を保つ。
for(const ground of [false,true]){
 const manual={...p,target:11,ground,distanceLOD:false};
 const selected=LOD.choose(roots,index,()=>true,manual,4,8);
 assert.equal(selected.leaves.length,256);
 assert.ok(selected.leaves.every(n=>n.row[0]===11));
 assert.equal(selected.coverageLimited,false);
 const items=selected.leaves.map(n=>({row:n.row,range:[0,0,1,1],lodN:LOD.mesh({row:n.row,range:[0,0,1,1]},manual)}));
 assert.equal(items.reduce((n,item)=>n+(item.lodN+1)**2,0),256*257**2);
 assert.ok(items.every(i=>i.lodN===256));
}
console.log('手動：指定レベルと分割数を維持 OK');

// 同じデータレベルでも場所によって分割を粗くすることはない。
for(const ground of [false,true])for(const tilt of [0,.6,1.3])for(const scale of [1000,1000000]){
 assert.equal(LOD.mesh({row:[12,2064,2064],range:[0,0,1,1]},{...p,ground,tilt,scale}),256);
}
console.log('距離・縮尺・傾斜による描画間引きなし OK');

// 地面の四隅は画面外でも、高い山の上部が手前から画面へ入る。
const span=LOD.world/2**12,edgeRow=[12,2064,2064,2000,.03];
const view={center:[2064.5*span,2064*span-3000],bearing:0,tilt:Math.PI/3,scale:1000,aspect:1.5,exaggeration:1};
assert.equal(LOD.overviewVisible([12,2064,2064,0,0],view),false);
assert.equal(LOD.overviewVisible(edgeRow,view),true);
assert.equal(LOD.overviewVisible(edgeRow,{...view,tilt:0}),false);
console.log('傾斜時は標高を含む表示範囲で手前の山を残す OK');

// 固定256分割でも、異なるデータレベル同士の境界に隙間を作らない。
const fixedCoarse={...coarse,lodN:256},fixedFine={...fine,lodN:256};
const fixedEdges=Edges.create([fixedCoarse,fixedFine],{base:7,target:14},sample);
const c256=new Float32Array(4*257*2),f256=new Float32Array(c256.length),grad256=new Float32Array(c256.length);
fixedEdges(fixedCoarse,256,c256,grad256);fixedEdges(fixedFine,256,f256,grad256);
for(let i=0;i<=256;i++){const t=i/2,cell=Math.min(255,Math.floor(t)),f=t-cell;assert.ok(Math.abs(f256[(3*257+i)*2]-(c256[(257+cell)*2]*(1-f)+c256[(257+cell+1)*2]*f))<.001);}
console.log('256分割固定の異なるデータレベルの境界接続 OK');
