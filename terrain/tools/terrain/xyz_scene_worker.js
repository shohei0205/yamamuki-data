'use strict';
importScripts('/xyz_terrain.js','/surface-methods.js');
onmessage=event=>{const {ticket,tiles,origin,divisions,join}=event.data;try{
 const started=performance.now(),mesh=XYZGeometry.merge(tiles,join),source=mesh.points;
 // LoopはXY平面と高さの順で計算し、描画時にY軸を上へ変換する。
 for(let i=0;i<source.length;i+=3){source[i]=(source[i]-origin[0])*XYZGeometry.UNIT;source[i+1]=(source[i+1]-origin[1])*XYZGeometry.UNIT}
 let actual=divisions;while(actual>1&&mesh.indices.length/3*actual**2>4000000)actual/=2;
 let plan=null,indices=mesh.indices,positions,normals;
 if(actual>1&&indices.length){const result=jointLoopSurface({points:source,indices,uncut:true},[],1000,actual,1,.5,true);plan=result.gpuLoopPlan;plan.options.quantum=1/1024;indices=result.indices;positions=new Float32Array(0);normals=new Float32Array(0)}else{positions=new Float32Array(source.length);for(let i=0;i<source.length;i+=3){positions[i]=source[i];positions[i+1]=source[i+2];positions[i+2]=source[i+1]}normals=XYZGeometry.normals(positions,indices)}
 const result={ticket,positions,normals,indices,plan,stats:{...mesh.stats,vertices:source.length/3,divisions:actual,triangles:indices.length/3,assemblySeconds:(performance.now()-started)/1000}};
 const buffers=new Set();function collect(v){if(ArrayBuffer.isView(v))buffers.add(v.buffer);else if(v&&typeof v==='object')for(const p of Object.values(v))collect(p)}collect(result);postMessage(result,[...buffers]);
 }catch(error){postMessage({ticket,error:error.message})}};
