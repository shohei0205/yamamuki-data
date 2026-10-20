const assert=require('node:assert/strict');
const {createTerrainHct,fitMinimumBendingEdges}=require('./hct-terrain.js');
const points=new Float32Array([0,0,0,1000,0,100,0,1000,50,1000,1000,300,450,400,170]);
const indices=new Uint32Array([0,1,4,1,3,4,3,2,4,2,0,4]);
const slopes=new Float32Array([.1,.05,.1,.2,.3,.05,.3,.2,-.1,.4]);
const fit=fitMinimumBendingEdges(points,indices,slopes,[0,1000,0,1000]);
assert.ok(fit.energyDelta< -1e-8);assert.ok(fit.residual<1e-5);
const build=ids=>createTerrainHct(ids.map(i=>Array.from(points.subarray(i*3,i*3+3))),ids.map(i=>Array.from(slopes.subarray(i*2,i*2+2))),[[ids[0],ids[1]],[ids[1],ids[2]],[ids[2],ids[0]]].map(([a,b])=>fit.gradients.get(a<b?`${a}/${b}`:`${b}/${a}`)));
const left=build([0,1,4]),right=build([1,3,4]);let max=0;
for(let i=0;i<=100;i++){const t=i/100,a=left(1-t,t),b=right(0,t);max=Math.max(max,Math.abs(a.height-b.height),...a.gradient.map((v,k)=>Math.abs(v-b.gradient[k])));}
assert.ok(max<1e-6);
const plane=new Float32Array(points);for(let i=0;i<5;i++)plane[i*3+2]=.2*plane[i*3]-.1*plane[i*3+1]+2;
const pg=new Float32Array(Array.from({length:5},()=>[.2,-.1]).flat());
const pf=fitMinimumBendingEdges(plane,indices,pg,[0,1000,0,1000]);assert.ok(Math.abs(pf.energyDelta)<1e-10);
console.log({energyDelta:fit.energyDelta,residual:fit.residual,iterations:fit.iterations,maxSharedDifference:max});
