const assert=require('node:assert/strict'),{createTerrainHct,fitTerrainSlopeDerivatives}=require('./hct-terrain.js'),{subdivideTerrain}=require('./pn-terrain.js');
const points=new Float32Array([0,0,20,1000,0,300,0,800,40,1000,800,700]),indices=new Uint32Array([0,1,2,1,3,2]),slopes=new Float32Array([.2,.1,-.1,.3,.05,-.2,.3,.1]),derivatives=fitTerrainSlopeDerivatives(points,indices,slopes);
let maxValue=0,maxDerivative=0;
for(let component=0;component<2;component++){
 const patch=[];for(let t=0;t<2;t++){const ids=Array.from(indices.subarray(t*3,t*3+3));patch.push(createTerrainHct(ids.map(i=>[points[i*3],points[i*3+1],slopes[i*2+component]]),ids.map(i=>Array.from(derivatives.subarray(i*4+component*2,i*4+component*2+2)))));}
 for(let j=0;j<=100;j++){const v=j/100,a=patch[0](1-v,v),b=patch[1](0,v);maxValue=Math.max(maxValue,Math.abs(a.height-b.height));maxDerivative=Math.max(maxDerivative,Math.hypot(a.gradient[0]-b.gradient[0],a.gradient[1]-b.gradient[1]));}
}
assert.ok(maxValue<1e-8);assert.ok(maxDerivative<1e-10);
const affine=new Float32Array(Array.from({length:4},(_,i)=>[.001*points[i*3]+.002*points[i*3+1]+.1,-.003*points[i*3]+.001*points[i*3+1]-.2]).flat()),affineDerivatives=fitTerrainSlopeDerivatives(points,indices,affine);for(let i=0;i<4;i++)for(let k=0;k<4;k++)assert.ok(Math.abs(affineDerivatives[i*4+k]-[.001,.002,-.003,.001][k])<1e-8);
const boundary=fitTerrainSlopeDerivatives(points,indices,slopes,[0,1000,0,800]);assert.ok([...boundary].every(v=>v===0));
const source={points,indices,vertexNormals:new Float32Array(12),bounds:[0,1000,0,800]},options={method:'fitted',slopes,curveBoundary:true,patchBuilder:createTerrainHct,interpolateNormals:true,analyticNormals:true},before=subdivideTerrain(source,4,1,1000,options),after=subdivideTerrain(source,4,1,1000,{...options,slopeDerivatives:derivatives});assert.deepEqual(after.points,before.points);assert.deepEqual(after.indices,before.indices);assert.equal(after.patchData.length,224);assert.ok([...after.patchData].every(Number.isFinite));assert.ok(after.patchData.some((v,i)=>i%112>=48&&Math.abs(v)>1e-9));
console.log('傾きの場：共有辺の値と変化率の連続性・線形場の再現・境界条件・形状不変を確認',{maxValue,maxDerivative});
