'use strict';
const{chromium}=require('playwright'),assert=require('node:assert/strict'),fs=require('node:fs');
(async()=>{
 const browser=await chromium.launch({channel:'chrome',headless:true,args:['--ignore-gpu-blocklist']});
 try{const page=await browser.newPage();await page.goto('http://127.0.0.1:61467/');await page.waitForFunction(()=>typeof BsplineSurface!=='undefined');
  const result=await page.evaluate(()=>{
   const canvas=document.createElement('canvas');canvas.width=canvas.height=1;const g=canvas.getContext('webgl2',{antialias:false});g.disable(g.DITHER);
   const shader=(type,source)=>{const s=g.createShader(type);g.shaderSource(s,source);g.compileShader(s);if(!g.getShaderParameter(s,g.COMPILE_STATUS))throw Error(g.getShaderInfoLog(s));return s;};
   const fragment=BsplineSurface.fragmentShader.replace('color=vec4(clamp(base*shade,0.,1.),1.);','uint bits=floatBitsToUint(length(position-eye));color=vec4(float(bits&255u),float((bits>>8u)&255u),float((bits>>16u)&255u),float((bits>>24u)&255u))/255.;');
   const program=g.createProgram();g.attachShader(program,shader(g.VERTEX_SHADER,BsplineSurface.vertexShader));g.attachShader(program,shader(g.FRAGMENT_SHADER,fragment));g.linkProgram(program);if(!g.getProgramParameter(program,g.LINK_STATUS))throw Error(g.getProgramInfoLog(program));g.useProgram(program);g.bindVertexArray(g.createVertexArray());
   const u=k=>g.getUniformLocation(program,k),n=128,halo=16,step=250,size=n+1+halo*2,texture=g.createTexture();g.bindTexture(g.TEXTURE_2D,texture);g.texParameteri(g.TEXTURE_2D,g.TEXTURE_MIN_FILTER,g.NEAREST);g.texParameteri(g.TEXTURE_2D,g.TEXTURE_MAG_FILTER,g.NEAREST);
   g.uniform1i(u('coefficients'),0);g.uniform1f(u('span'),32000);g.uniform1f(u('stepSize'),step);g.uniform1i(u('halo'),halo);g.uniform4f(u('viewport'),0,0,1,1);g.uniform2f(u('rayScale'),0,0);g.uniform1f(u('nearPlane'),1);g.uniformMatrix4fv(u('matrix'),false,new Float32Array([1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1]));
   const cases=[
    {name:'parallel-above-5cm',height:1000.05,dy:0,slope:0,expected:null},
    {name:'parallel-below-5cm',height:999.95,dy:0,slope:0,expected:null},
    {name:'parallel-above-1mm',height:1000.001,dy:0,slope:0,expected:null},
    {name:'grazing-miss',height:1000.05,dy:-.000001,slope:0,expected:null},
    {name:'downward-plane',height:1100,dy:-.1,slope:0,expected:1000},
    {name:'near-grazing-plane',height:1000.05,dy:-.001,slope:0,expected:50},
    {name:'parallel-to-slope',height:1000.05,dy:0,slope:.01,expected:5/Math.sqrt(1-.0001)},
   ];
   const rows=[];
   for(const c of cases){const controls=new Float32Array(size*size);for(let y=0;y<size;y++)for(let x=0;x<size;x++)controls[y*size+x]=1000-c.slope*((y-halo)*step-16000);g.texImage2D(g.TEXTURE_2D,0,g.R32F,size,size,0,g.RED,g.FLOAT,controls);g.uniform2f(u('gradientBounds'),0,Math.abs(c.slope));g.uniform2f(u('heightBounds'),c.slope?800:999.9,c.slope?1200:1000.1);g.uniform3f(u('eye'),0,c.height,0);g.uniform3f(u('back'),0,-c.dy,Math.sqrt(1-c.dy*c.dy));g.uniform3f(u('right'),1,0,0);g.uniform3f(u('up'),0,1,0);g.clearColor(0,0,0,0);g.clear(g.COLOR_BUFFER_BIT);g.drawArrays(g.TRIANGLES,0,3);const pixel=new Uint8Array(4);g.readPixels(0,0,1,1,g.RGBA,g.UNSIGNED_BYTE,pixel);const distance=new DataView(pixel.buffer).getFloat32(0,true);rows.push({...c,actual:pixel.every(v=>v===0)?null:distance});}return {rows,error:g.getError()};
  });
  console.log(JSON.stringify(result));fs.writeFileSync('build/bspline-surface-prototype-v3/intersection-check.json',JSON.stringify(result,null,2));assert.equal(result.error,0);
  for(const row of result.rows){if(row.expected===null)assert.equal(row.actual,null,row.name);else{assert.notEqual(row.actual,null,row.name);assert(Math.abs(row.actual-row.expected)<.15,`${row.name}: ${row.actual}`);}}
 }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exit(1);});
