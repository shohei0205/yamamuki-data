'use strict';
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path');
const {cubicRoot}=require('./lpf_curve_raycast.js');
const close=(actual,expected,tolerance=1e-8)=>assert(Math.abs(actual-expected)<tolerance,`${actual} != ${expected}`);
close(cubicRoot([1,-1.3,.5,-.056]),.2);
close(cubicRoot([1,-1.3,.5,-.056],.3,1),.4);
close(cubicRoot([1,-1.3,.5,-.056],.5,1),.7);
close(cubicRoot([0,1,-.8,.16]),.4);
close(cubicRoot([0,0,2,-1]),.5);
close(cubicRoot([1,0,0,0]),0);
assert.equal(cubicRoot([0,1,-.8,.26]),null);
assert.equal(cubicRoot([0,0,0,1]),null);
const {chromium}=require('playwright');
(async()=>{
 const output=path.resolve(process.argv[2]);fs.mkdirSync(output,{recursive:true});
 const browser=await chromium.launch({channel:'chrome',headless:true,args:['--use-angle=swiftshader','--enable-unsafe-swiftshader']});
 try{
  const page=await browser.newPage({viewport:{width:1400,height:900}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.goto('http://127.0.0.1:61465/?region=fuji&z=6&renderer=ray');await page.waitForFunction(()=>window.lpfCurveStatus?.loaded);
  const gpu=await page.evaluate(()=>{
   const canvas=document.createElement('canvas');canvas.width=canvas.height=1;
   const g=canvas.getContext('webgl2',{antialias:false,alpha:true,premultipliedAlpha:false,preserveDrawingBuffer:true});
   const engine=LpfCurveRaycast.create(g,'distance'),vao=g.createVertexArray();g.bindVertexArray(vao);
   const buffer=g.createBuffer();g.bindBuffer(g.ARRAY_BUFFER,buffer);
   for(let i=1;i<=4;i++){g.enableVertexAttribArray(i);g.vertexAttribPointer(i,4,g.FLOAT,false,64,(i-1)*16);g.vertexAttribDivisor(i,1);}
   g.enable(g.DEPTH_TEST);g.clearColor(0,0,0,0);g.viewport(0,0,1,1);
   const rows=[];
   function test(name,records,pose,expected,tolerance=.02){
    g.bindVertexArray(vao);g.bindBuffer(g.ARRAY_BUFFER,buffer);g.bufferData(g.ARRAY_BUFFER,new Float32Array(records.flat()),g.STATIC_DRAW);
    g.clear(g.COLOR_BUFFER_BIT|g.DEPTH_BUFFER_BIT);Object.assign(camera,pose);
    engine.draw(matrix(1),camera,[0,0,1,1],vao,records.length,false);
    const color=new Uint8Array(4);g.readPixels(0,0,1,1,g.RGBA,g.UNSIGNED_BYTE,color);
    const value=color.every(v=>v===0)?null:new DataView(color.buffer).getFloat32(0,true);
    const error=g.getError();if(error)throw Error(`${name}: WebGL ${error}`);
    if(expected===null?value!==null:(value===null||Math.abs(value-expected)>tolerance))throw Error(`${name}: ${value} != ${expected}`);
    rows.push({name,value,expected});
   }
   const triangle=[-100,-100,100,-100,0,100],flat=h=>[...triangle,...Array(10).fill(h)];
   const vertical={x:0,y:0,height:0,distance:10,yaw:0,pitch:Math.PI/2};
   test('平面の交点',[flat(5)],vertical,5);
   test('深度・奥から手前',[flat(2),flat(5)],vertical,5);
   test('深度・手前から奥',[flat(5),flat(2)],vertical,5);
   test('近面より手前は非表示',[flat(9.5)],vertical,null);
   const exponent=[0,3,0,1,2,0,0,2,1,1];
   const cubic=[-.056,.11066666666666666,-.156,.144];
   const horizontal={x:100,y:10,height:0,distance:100,yaw:-Math.PI/2,pitch:0};
   test('3交点のうち最も手前',[[0,0,100,0,0,100,...exponent.map(j=>cubic[j]*100)]],horizontal,20);
   test('逆向きの最も手前',[[0,0,100,0,0,100,...exponent.map(j=>cubic[j]*100)]],{...horizontal,x:-10,yaw:Math.PI/2},20);
   const tangent=[.16,.16-.8/3,.16-1.6/3+1/3,.36];
   test('接するだけの交点',[[0,0,100,0,0,100,...exponent.map(j=>tangent[j]*100)]],horizontal,40,.05);
   test('交点なし',[[0,0,100,0,0,100,...exponent.map(j=>(tangent[j]+.1)*100)]],horizontal,null);
   const plane=[-95,105,5,-95+200/3,-95+400/3,-95+100/3,-95+200/3,105-100/3,105-200/3,5];
   test('補助プリズムの中から表示',[[...triangle,...plane]],vertical,5);
   test('共有辺上の画素',[
    [-100,-100,100,-100,100,100,...Array(10).fill(5)],
    [-100,-100,100,100,-100,100,...Array(10).fill(5)]],vertical,5);
   g.getExtension('WEBGL_lose_context')?.loseContext();
   return rows;
  });
  assert.deepEqual(errors,[]);
  const report={passed:true,cpuCases:8,gpuCases:gpu,errors};fs.writeFileSync(path.join(output,'intersection-verification.json'),JSON.stringify(report,null,2));
  console.log(JSON.stringify({passed:true,cpuCases:8,gpuCases:gpu.length}));
 }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
