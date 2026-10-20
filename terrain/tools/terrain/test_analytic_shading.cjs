const assert=require('node:assert/strict'),{createTerrainHct}=require('./hct-terrain.js'),{subdivideTerrain}=require('./pn-terrain.js');
const points=new Float32Array([0,0,30,1000,0,150,0,800,-40,1000,800,200]),slopes=new Float32Array([.2,.1,-.1,.3,.05,-.2,.3,.1]),indices=new Uint32Array([0,1,2,1,3,2]),source={points,indices,vertexNormals:new Float32Array([0,0,1,0,0,1,0,0,1,0,0,1]),bounds:[0,1000,0,800]};
let maxGeometryDifference=0,maxGradientDifference=0;
for(const divisions of [2,4,8])for(const strength of [.5,1]){
 const options={method:'fitted',slopes,curveBoundary:true,patchBuilder:createTerrainHct,interpolateNormals:true},old=subdivideTerrain(source,divisions,strength,1000,options),next=subdivideTerrain(source,divisions,strength,1000,{...options,analyticNormals:true});
 assert.equal(next.indices.length,old.indices.length);assert.equal(next.patchData.length,224);assert.equal(next.patchIds.length,next.points.length/3);assert.equal(next.patchUV.length,next.points.length/3*2);
 for(let i=0;i<next.indices.length;i++)for(let k=0;k<3;k++)maxGeometryDifference=Math.max(maxGeometryDifference,Math.abs(next.points[next.indices[i]*3+k]-old.points[old.indices[i]*3+k]));
 for(let i=0;i<next.indices.length;i+=3){const ids=next.indices.subarray(i,i+3);assert.equal(next.patchIds[ids[0]],next.patchIds[ids[1]]);assert.equal(next.patchIds[ids[1]],next.patchIds[ids[2]]);}
 for(let t=0;t<2;t++){const ids=Array.from(indices.subarray(t*3,t*3+3)),p=ids.map(i=>Array.from(points.subarray(i*3,i*3+3))),g=ids.map(i=>Array.from(slopes.subarray(i*2,i*2+2))),f=createTerrainHct(p,g),data=next.patchData.subarray(t*112,t*112+112);assert.equal(data[38],strength);assert.deepEqual(Array.from(data.subarray(40,46)),g.flat());
 for(let i=0;i<=20;i++)for(let j=0;j<=20-i;j++){const u=i/20,v=j/20,w=1-u-v,k=(v<=u&&v<=w?0:w<=u&&w<=v?1:2)*10,c=data.subarray(k,k+10),du=c[1]+2*c[3]*u+c[4]*v+3*c[6]*u*u+2*c[7]*u*v+c[8]*v*v,dv=c[2]+c[4]*u+2*c[5]*v+c[7]*u*u+2*c[8]*u*v+3*c[9]*v*v;const gx=du*data[32]+dv*data[33],gy=du*data[34]+dv*data[35],ref=f(u,v).gradient;maxGradientDifference=Math.max(maxGradientDifference,Math.abs(gx-ref[0]),Math.abs(gy-ref[1]));}}
}
assert.ok(maxGeometryDifference<1e-4);assert.ok(maxGradientDifference<5e-5);console.log('画素法線用係数・三角形単位の参照・UV・既存形状の維持を確認',{maxGeometryDifference,maxGradientDifference});
