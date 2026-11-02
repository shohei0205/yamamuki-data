// 高さが違う平面を接合しても、陰影に人工的な稜線を作らないことを実GPUで検証する。
const {chromium}=require('playwright');
const assert=require('node:assert/strict');
(async()=>{const b=await chromium.launch({channel:'chrome',headless:true});try{
 const p=await b.newPage();await p.goto('http://127.0.0.1:61491/control-gpu128-render.wgsl');
 for(const coarse of [false,true]) {
 const rows=await p.evaluate(async(coarse)=>{
  const adapter=await navigator.gpu.requestAdapter(),device=await adapter.requestDevice();
  device.pushErrorScope('validation');
  const code=await (await fetch('/control-gpu128-render.wgsl')).text();
  const pipeline=await device.createComputePipelineAsync({layout:'auto',compute:{module:device.createShaderModule({code}),entryPoint:'checkSeams'}});
  const buffer=(data,usage)=>{const b=device.createBuffer({size:Math.max(4,data.byteLength),usage:usage|GPUBufferUsage.COPY_DST});device.queue.writeBuffer(b,0,data);return b};
  const values=new Float32Array(8*8);for(let i=0;i<8;i++)for(let j=0;j<2;j++){values[i*8+j*4]=i<4?10:70;values[i*8+j*4+1]=coarse?(i<4?.02:.04):0;values[i*8+j*4+3]=1;}
  const tileData=new ArrayBuffer(128),f=new Float32Array(tileData),u=new Uint32Array(tileData);
  const side=coarse?100000:1000;
  for(let i=0;i<2;i++){const k=i*16;f.set([i*side,0,(i+1)*side,side],k);f.set([i*side-250,-250,(i+1)*side+250,side+250],k+4);u.set([i*4,1,i*2,2],k+8);f.set([60,16,0,0],k+12);}
  const camera=new ArrayBuffer(64);new Uint32Array(camera).set([1,0,0,0],12);new Float32Array(camera)[9]=.25;
  const positions=coarse?[side-1500,side-600,side-1,side,side,side+1,side+600,side+1500]:[800,900,999,1000,1000,1001,1100,1200];const queries=new Float32Array(positions.flatMap((x,i)=>[x,side/2,i<4?0:1,0]));
  const inputs=[buffer(values,GPUBufferUsage.STORAGE),buffer(tileData,GPUBufferUsage.STORAGE),buffer(new Uint32Array([0,1,0,1]),GPUBufferUsage.STORAGE),buffer(camera,GPUBufferUsage.UNIFORM),buffer(queries,GPUBufferUsage.STORAGE)];
  const output=device.createBuffer({size:queries.byteLength,usage:GPUBufferUsage.STORAGE|GPUBufferUsage.COPY_SRC}),read=device.createBuffer({size:queries.byteLength,usage:GPUBufferUsage.COPY_DST|GPUBufferUsage.MAP_READ});
  const bind=device.createBindGroup({layout:pipeline.getBindGroupLayout(0),entries:[...inputs,output].map((b,binding)=>({binding,resource:{buffer:b}})).concat([{binding:7,resource:{buffer:inputs[0]}}])});const encoder=device.createCommandEncoder(),pass=encoder.beginComputePass();pass.setPipeline(pipeline);pass.setBindGroup(0,bind);pass.dispatchWorkgroups(1);pass.end();encoder.copyBufferToBuffer(output,0,read,0,queries.byteLength);device.queue.submit([encoder.finish()]);await read.mapAsync(GPUMapMode.READ);const error=await device.popErrorScope();if(error)throw Error(error.message);const a=new Float32Array(read.getMappedRange());const result=positions.map((x,i)=>({x,height:a[i*4],dx:a[i*4+1],dy:a[i*4+2]}));read.unmap();for(const b of [...inputs,output,read])b.destroy();device.destroy();return result;
 },coarse);
 if(coarse){assert(rows[1].dx>.0201&&rows[1].dx<.04,JSON.stringify(rows));assert(rows[6].dx>.02&&rows[6].dx<.0399,JSON.stringify(rows));assert(Math.abs(rows[3].dx-rows[4].dx)<1e-6);assert(Math.abs(rows[3].height-rows[4].height)<1e-5);console.log('粗いキャッシュ：狭い余白を越えて法線を連続接合',rows);continue;}
 assert(rows.every(v=>Math.abs(v.dx)<1e-6&&Math.abs(v.dy)<1e-6),JSON.stringify(rows));assert(rows.every(v=>v.height>=10-1e-4&&v.height<=70+1e-4),JSON.stringify(rows));for(let i=1;i<rows.length;i++)assert(rows[i].height>=rows[i-1].height-1e-5);assert(Math.abs(rows[3].height-rows[4].height)<1e-5);console.log('高さ差60 mの平面接合：形状の連続性を保ち、陰影の人工的な筋なし',rows);
 }
}finally{await b.close()}})().catch(e=>{console.error(e);process.exitCode=1});
