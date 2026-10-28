'use strict';
const {chromium}=require('playwright'),assert=require('node:assert/strict'),fs=require('node:fs');
(async()=>{const b=await chromium.launch({channel:'chrome',headless:true,args:['--ignore-gpu-blocklist']});const rows=[],errors=[],checks=[];try{
 const page=await b.newPage({viewport:{width:1200,height:800}});page.on('pageerror',e=>errors.push(e.message));
 for(const lpf of (process.env.CONTINUOUS_SPLINE ? [0] : [1,0]))for(const region of ['yari','fuji','tsukuba']){
  await page.goto(`http://127.0.0.1:${process.env.CONTINUOUS_SPLINE ? 61477 : 61476}/?region=${region}&keep=1&lpf=${lpf}`);
  for(const keep of [1,.5,.25,.125,.0625]){
   if(keep!==1)await page.selectOption('#pruneKeep',String(keep));
   await page.waitForFunction(([r,k,l])=>window.lpfCurveStatus?.renderer==='prunedSpline'&&lpfCurveStatus.spline?.region===r&&lpfCurveStatus.spline.keep===k&&lpfCurveStatus.spline.lpfEnabled===!!l,[region,keep,lpf],{timeout:120000});
   const value=await page.evaluate(async({region,keep,lpf})=>{
    const m=await(await fetch(`/api/pruned-spline?region=${region}&keep=${keep}&lpf=${lpf}`)).json(),c=new Float32Array(Uint8Array.from(atob(m.coefficients),c=>c.charCodeAt(0)).buffer),ref=new Float32Array(Uint8Array.from(atob(m.reference.coefficients),c=>c.charCodeAt(0)).buffer),points=new Float32Array(Uint8Array.from(atob(m.controlPoints),c=>c.charCodeAt(0)).buffer);
    const basis=t=>[(1-t)**3/6,(3*t**3-6*t*t+4)/6,(-3*t**3+3*t*t+3*t+1)/6,t**3/6];
    function at(co,x){const qx=(x+16000)/125,qy=128,ix=Math.floor(qx)+2,iy=130,bx=basis(qx-Math.floor(qx)),by=basis(0);let h=0;for(let j=0;j<4;j++)for(let i=0;i<4;i++)h+=co[(iy+j-1)*261+ix+i-1]*bx[i]*by[j];return h;}
    const xs=[-15000,-4000,0,6000,15000],actual=splineEngine.profile(xs),original=referenceSplineEngine.profile(xs);let profileError=0,referenceError=0,fullError=0;for(let i=0;i<xs.length;i++){profileError=Math.max(profileError,Math.abs(actual[i]-at(c,xs[i])));referenceError=Math.max(referenceError,Math.abs(original[i]-at(ref,xs[i])));fullError=Math.max(fullError,Math.abs(actual[i]-original[i]));}
    let pixelError=0;if(keep===1){draw();const w=canvas.width,h=canvas.height,p=new Uint8Array(w*h*4);gl.readPixels(0,0,w,h,gl.RGBA,gl.UNSIGNED_BYTE,p);for(let y=0;y<h;y+=7)for(let x=0;x<w/2;x+=7)for(let channel=0;channel<3;channel++)pixelError=Math.max(pixelError,Math.abs(p[(y*w+x)*4+channel]-p[(y*w+x+w/2)*4+channel]));}
    return{region,keep,lpf,profileError,referenceError,fullError,pixelError,controlCount:points.length/3,displayControlCount:splineEngine.pointStats.controls,error:gl.getError(),bytes:m.info.cumulativeBytes,originalBytes:m.info.originalBytes,refit:m.info.refit,camera:{...camera}};
   },{region,keep,lpf});
   console.log(JSON.stringify(value));assert.equal(value.error,0);assert(value.profileError<.001);assert(value.referenceError<.001);assert.equal(value.controlCount,value.displayControlCount);assert(value.refit);if(keep===1){assert(value.fullError<.001);assert(value.pixelError<=2);}else{assert(value.bytes<value.originalBytes);assert.deepEqual(value.camera,rows.at(-1).camera);}rows.push(value);
   if(keep===.25||keep===.0625)await page.screenshot({path:`build/${process.env.CONTINUOUS_SPLINE ? "spline-pruning-continuous-v1" : "spline-pruning-v2"}/${region}-${keep}.png`});
  }
  await page.locator('#splineControlPoints').check();assert.equal(await page.evaluate(()=>gl.getError()),0);await page.locator('#splineControlPoints').uncheck();await page.locator('#normals').uncheck();assert.equal(await page.evaluate(()=>gl.getError()),0);await page.locator('#normals').check();
  await page.selectOption('#renderer','ridgeSpline');await page.waitForFunction(()=>lpfCurveStatus.renderer==='ridgeSpline'&&!lpfCurveStatus.spline?.pruning);await page.selectOption('#renderer','prunedSpline');await page.waitForFunction(()=>lpfCurveStatus.renderer==='prunedSpline'&&lpfCurveStatus.spline?.keep===.0625);checks.push({region,lpf,points:true,flat:true,originalSwitch:true});
 }
 assert.deepEqual(errors,[]);fs.writeFileSync(process.env.CONTINUOUS_SPLINE ? 'build/spline-pruning-continuous-v1/browser-verification.json' : 'build/spline-pruning-v2/browser-verification.json',JSON.stringify({passed:true,rows,checks,errors},null,2));
}finally{await b.close();}})().catch(e=>{console.error(e);process.exit(1);});
