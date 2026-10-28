'use strict';
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{
 const out=path.resolve(process.argv[2]||'build/bspline-surface-prototype-v3/polygon-browser-check');fs.mkdirSync(out,{recursive:true});
 const browser=await chromium.launch({channel:'chrome',headless:true,args:['--ignore-gpu-blocklist']});
 const page=await browser.newPage({viewport:{width:1200,height:800}}),errors=[],rows=[];
 page.on('pageerror',e=>errors.push(e.message));
 async function ready(region,n){await page.waitForFunction(([r,n])=>window.lpfCurveStatus?.loaded&&lpfCurveStatus.region===r&&lpfCurveStatus.spline?.n===n,[region,n],{timeout:120000});const s=await page.evaluate(()=>lpfCurveStatus);assert.equal(s.error,0);assert.equal(s.surfaceTriangles,2*(n*s.splineMesh.division)**2);assert(s.surfaceTriangles>0);assert.equal(s.proxyTriangles,0);assert.equal(s.renderer,'bspline');return s;}
 try{
  for(const region of ['fuji','yari','tsukuba']){
   await page.goto(`http://127.0.0.1:61467/?region=${region}&n=8&splineDebug=1`);rows.push(await ready(region,8));
   for(const n of [128,512]){await page.selectOption('#splineQuality',String(n));rows.push(await ready(region,n));}
   await page.screenshot({path:path.join(out,`${region}-n512.png`)});
   assert(await page.locator('#divisions').isEnabled());assert(await page.locator('#normals').isEnabled());
   await page.selectOption('#splineQuality','128');await ready(region,128);
   for(const d of [1,2,4,8]){await page.selectOption('#divisions',String(d));const v=await ready(region,128);assert.equal(v.splineMesh.division,d);}
   await page.locator('#normals').uncheck();assert.equal((await ready(region,128)).error,0);await page.locator('#normals').check();
   await page.locator('#wire').check();assert.equal((await ready(region,128)).error,0);await page.locator('#wire').uncheck();
   await page.selectOption('#divisions','4');
   // 低い視点・接近・回転中も実際のポリゴンが描かれることを確認する。
   for(const quality of [128,512]){
   await page.selectOption('#splineQuality',String(quality));await ready(region,quality);
   const checks=await page.evaluate(()=>{
    const g=document.getElementById('terrain').getContext('webgl2'),c=document.getElementById('terrain'),rows=[];
    for(const pitch of [1.1,.5,.15])for(const distance of [8000,30000]){camera.pitch=pitch;camera.distance=distance;camera.yaw=distance===8000?.7:0;draw();const bytes=new Uint8Array(c.width*c.height*4);g.readPixels(0,0,c.width,c.height,g.RGBA,g.UNSIGNED_BYTE,bytes);let terrainPixels=0;for(let y=0;y<c.height;y++)for(let x=c.width/2|0;x<c.width;x++){const i=(y*c.width+x)*4;if(bytes[i+1]<220)terrainPixels++;}rows.push({pitch,distance,terrainPixels,error:g.getError()});}return rows;
   });
   fs.writeFileSync(path.join(out,`${region}-n${quality}-polygon-check.json`),JSON.stringify(checks,null,2));console.log(region,JSON.stringify(checks));assert(checks.every(v=>v.error===0&&v.terrainPixels>100));}
   await page.evaluate(()=>{camera.pitch=.3;camera.distance=16000;camera.yaw=.7;draw();});
   await page.screenshot({path:path.join(out,`${region}-low-view.png`)});
   // 以前の曲面と三角形描画も利用できる。
   await page.selectOption('#renderer','ray');assert.equal((await page.evaluate(()=>lpfCurveStatus)).renderer,'ray');
   await page.selectOption('#renderer','triangles');assert.equal((await page.evaluate(()=>lpfCurveStatus)).renderer,'triangles');
  }
  const fixture=JSON.parse(fs.readFileSync(path.join(out,'../gpu-field-fixture.json'),'utf8'));
  const gpuField=await page.evaluate(async fixture=>{
   const canvas=document.createElement('canvas');canvas.width=canvas.height=1;const g=canvas.getContext('webgl2',{antialias:false});
   const shader=(type,source)=>{const s=g.createShader(type);g.shaderSource(s,source);g.compileShader(s);if(!g.getShaderParameter(s,g.COMPILE_STATUS))throw Error(g.getShaderInfoLog(s));return s;};
   const fs=BsplineSurface.polygonFragmentShader.split('void main')[0]+`uniform vec2 testPoint;uniform int component;void main(){vec3 v=field(testPoint);uint b=floatBitsToUint(v[component]);color=vec4(float(b&255u),float((b>>8u)&255u),float((b>>16u)&255u),float((b>>24u)&255u))/255.;}`;
   const program=g.createProgram();g.attachShader(program,shader(g.VERTEX_SHADER,BsplineSurface.vertexShader));g.attachShader(program,shader(g.FRAGMENT_SHADER,fs));g.linkProgram(program);if(!g.getProgramParameter(program,g.LINK_STATUS))throw Error(g.getProgramInfoLog(program));g.useProgram(program);g.disable(g.DITHER);const fieldVAO=g.createVertexArray();g.bindVertexArray(fieldVAO);
   const loc=k=>g.getUniformLocation(program,k),texture=g.createTexture();g.bindTexture(g.TEXTURE_2D,texture);g.texParameteri(g.TEXTURE_2D,g.TEXTURE_MIN_FILTER,g.NEAREST);g.texParameteri(g.TEXTURE_2D,g.TEXTURE_MAG_FILTER,g.NEAREST);
   const vertexProgram=g.createProgram(),vertexSource=BsplineSurface.polygonVertexShader.replaceAll('gl_InstanceID','testInstance').replace('void main(){','uniform int testInstance;void main(){');
   g.attachShader(vertexProgram,shader(g.VERTEX_SHADER,vertexSource));g.attachShader(vertexProgram,shader(g.FRAGMENT_SHADER,'#version 300 es\nprecision highp float;out vec4 color;void main(){color=vec4(1.);}'));g.transformFeedbackVaryings(vertexProgram,['position'],g.INTERLEAVED_ATTRIBS);g.linkProgram(vertexProgram);if(!g.getProgramParameter(vertexProgram,g.LINK_STATUS))throw Error(g.getProgramInfoLog(vertexProgram));
   const vertexLoc=k=>g.getUniformLocation(vertexProgram,k),vertexVAO=g.createVertexArray(),gridBuffer=g.createBuffer(),feedback=g.createBuffer();g.bindVertexArray(vertexVAO);g.bindBuffer(g.ARRAY_BUFFER,gridBuffer);g.bufferData(g.ARRAY_BUFFER,8,g.DYNAMIC_DRAW);g.enableVertexAttribArray(0);g.vertexAttribPointer(0,2,g.FLOAT,false,8,0);g.bindBuffer(g.TRANSFORM_FEEDBACK_BUFFER,feedback);g.bufferData(g.TRANSFORM_FEEDBACK_BUFFER,12,g.DYNAMIC_READ);g.bindBufferBase(g.TRANSFORM_FEEDBACK_BUFFER,0,feedback);g.bindVertexArray(fieldVAO);
   let maxHeightError=0,maxGradientError=0,count=0,vertexCount=0,maxVertexHeightError=0,maxVertexXYError=0;
   for(const entry of fixture){const m=await(await fetch(`/api/spline?region=${entry.region}&n=${entry.n}`)).json(),bytes=Uint8Array.from(atob(m.coefficients),v=>v.charCodeAt(0));g.texImage2D(g.TEXTURE_2D,0,g.R32F,m.size,m.size,0,g.RED,g.FLOAT,new Float32Array(bytes.buffer));g.uniform1i(loc('coefficients'),0);g.uniform1f(loc('span'),m.span);g.uniform1f(loc('stepSize'),m.info.stepMeters);g.uniform1i(loc('halo'),m.halo);
    g.useProgram(vertexProgram);g.uniform1i(vertexLoc('coefficients'),0);g.uniform1f(vertexLoc('span'),m.span);g.uniform1f(vertexLoc('stepSize'),m.info.stepMeters);g.uniform1i(vertexLoc('halo'),m.halo);g.uniform1i(vertexLoc('cells'),m.info.n);g.uniformMatrix4fv(vertexLoc('matrix'),false,new Float32Array([1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1]));g.useProgram(program);
    for(const p of entry.points){g.uniform2f(loc('testPoint'),p[0],p[1]);for(let k=0;k<3;k++){g.uniform1i(loc('component'),k);g.drawArrays(g.TRIANGLES,0,3);const pixel=new Uint8Array(4);g.readPixels(0,0,1,1,g.RGBA,g.UNSIGNED_BYTE,pixel);const v=new DataView(pixel.buffer).getFloat32(0,true),error=Math.abs(v-p[k+2]);if(!Number.isFinite(v))throw Error('GPU評価が非数');if(k===0)maxHeightError=Math.max(maxHeightError,error);else maxGradientError=Math.max(maxGradientError,error);count++;}
     const qx=(p[0]+m.span/2)/m.info.stepMeters,qy=(p[1]+m.span/2)/m.info.stepMeters,ix=Math.floor(qx),iy=Math.floor(qy);g.useProgram(vertexProgram);g.bindVertexArray(vertexVAO);g.bindBuffer(g.ARRAY_BUFFER,gridBuffer);g.bufferSubData(g.ARRAY_BUFFER,0,new Float32Array([qx-ix,qy-iy]));g.uniform1i(vertexLoc('testInstance'),iy*m.info.n+ix);g.enable(g.RASTERIZER_DISCARD);g.beginTransformFeedback(g.POINTS);g.drawArrays(g.POINTS,0,1);g.endTransformFeedback();g.disable(g.RASTERIZER_DISCARD);g.bindBuffer(g.TRANSFORM_FEEDBACK_BUFFER,feedback);const v=new Float32Array(3);g.getBufferSubData(g.TRANSFORM_FEEDBACK_BUFFER,0,v);maxVertexHeightError=Math.max(maxVertexHeightError,Math.abs(v[1]-p[2]));maxVertexXYError=Math.max(maxVertexXYError,Math.abs(v[0]-p[0]),Math.abs(v[2]-p[1]));vertexCount++;g.useProgram(program);g.bindVertexArray(fieldVAO);
    }
   }return {count,maxHeightError,maxGradientError,vertexCount,maxVertexHeightError,maxVertexXYError,error:g.getError()};
  },fixture);
  console.log(JSON.stringify(gpuField));assert.equal(gpuField.error,0);assert(gpuField.maxHeightError<.01);assert(gpuField.maxGradientError<.0001);assert(gpuField.maxVertexHeightError<.02);assert(gpuField.maxVertexXYError<.005);
  assert.deepEqual(errors,[]);fs.writeFileSync(path.join(out,'verification.json'),JSON.stringify({passed:true,rows,gpuField,errors},null,2));console.log(JSON.stringify({passed:true,cases:rows.length,gpuField,errors}));
 }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exit(1);});
