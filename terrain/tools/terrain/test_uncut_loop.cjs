const fs=require('fs'),vm=require('vm'),assert=require('node:assert/strict');
const context=vm.createContext({console,Float32Array,Float64Array,Uint8Array,Uint32Array,Int32Array,Map,Set,Math,performance});
vm.runInContext(fs.readFileSync('tools/terrain/surface-methods.js','utf8'),context);
const meshes=JSON.parse(fs.readFileSync('build/uncut-loop-fixture.json','utf8'));
for(const divisions of [2,4,8]){
 const groups=new Map();let shared=0,maxHeight=0,maxNormal=0,maxXY=0,faceShared=0,maxFaceAngle=0;const planeGroups=new Map();
 for(const mesh of meshes){
  const source={points:Float32Array.from(mesh.points),indices:Uint32Array.from(mesh.indices),referenceCheck:true};
  const result=context.jointLoopSurface(source,[{points:source.points,indices:Uint32Array.from(mesh.halo),dx:0,dy:0}],8000,divisions,1,.5,false);
  const p=result.points,n=result.normals,refs=result.referenceCheck;
  const refKey=i=>(refs[i*2]+mesh.dx*8000).toFixed(4)+'/'+refs[i*2+1].toFixed(4);
  for(let i=0;i<p.length/3;i++){
   const key=refKey(i),value=[p[i*3]+mesh.dx*8000,p[i*3+1],p[i*3+2],...n.subarray(i*3,i*3+3)];
   if(groups.has(key)){shared++;const old=groups.get(key);maxHeight=Math.max(maxHeight,Math.abs(old[2]-value[2]));maxXY=Math.max(maxXY,Math.abs(old[0]-value[0]),Math.abs(old[1]-value[1]));for(let k=3;k<6;k++)maxNormal=Math.max(maxNormal,Math.abs(old[k]-value[k]));}else groups.set(key,value);
  }
  for(let t=0;t<result.indices.length;t+=3){const ids=Array.from(result.indices.subarray(t,t+3)),key=ids.map(refKey).sort().join('|'),a=ids.map(i=>Array.from(p.subarray(i*3,i*3+3))),u=a[1].map((v,k)=>v-a[0][k]),v=a[2].map((v,k)=>v-a[0][k]),cross=[u[1]*v[2]-u[2]*v[1],u[2]*v[0]-u[0]*v[2],u[0]*v[1]-u[1]*v[0]],length=Math.hypot(...cross),normal=cross.map(v=>v/length);
   if(planeGroups.has(key)){faceShared++;const old=planeGroups.get(key);maxFaceAngle=Math.max(maxFaceAngle,Math.acos(Math.min(1,Math.max(-1,old.reduce((s,v,k)=>s+v*normal[k],0))))*180/Math.PI);}else planeGroups.set(key,normal);
  }
 }
 console.log({divisions,shared,maxHeight,maxXY,maxNormal,faceShared,maxFaceAngle});
 assert.ok(shared>100);assert.ok(faceShared>100);assert.ok(maxHeight<.01);assert.ok(maxXY<.01);assert.ok(maxNormal<.003);assert.ok(maxFaceAngle<1);
}
