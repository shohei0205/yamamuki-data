'use strict';
const {chromium}=require('playwright'),assert=require('node:assert/strict'),fs=require('node:fs');
(async()=>{
 const browser=await chromium.launch({channel:'chrome',headless:true,args:['--ignore-gpu-blocklist']});
 try{
  const page=await browser.newPage({viewport:{width:1200,height:800}}),errors=[],rows=[],checks=[];
  page.on('pageerror',e=>errors.push(e.message));
  const ready=async(region,budget)=>{await page.waitForFunction(([r,b])=>window.lpfCurveStatus?.adaptive?.region===r&&lpfCurveStatus.adaptive.budget===b,[region,budget],{timeout:120000});const v=await page.evaluate(()=>lpfCurveStatus);assert.equal(v.error,0);assert.equal(v.renderer,'adaptive');return v;};
  for(const region of ['yari','fuji','tsukuba']){
   await page.goto(`http://127.0.0.1:61473/?region=${region}&points=512&renderer=adaptive&layout=legacy`);rows.push(await ready(region,512));
   for(const budget of [2048,8192,32768]){await page.selectOption('#adaptiveBudget',String(budget));rows.push(await ready(region,budget));}
   await page.locator('#ridgeFocus').click();await page.screenshot({path:`build/adaptive-free-surface-v6/${region}-centers.png`});
   const check=await page.evaluate(async region=>{
    const m=await(await fetch(`/api/adaptive?region=${region}&budget=32768&layout=legacy`)).json(),features=new Float32Array(Uint8Array.from(atob(m.features),c=>c.charCodeAt(0)).buffer),g=(adaptiveEngine.setFairing(0),adaptiveEngine.readGeometry()),centers=adaptiveEngine.readControls();let maxHeightError=0,maxGradientError=0,maxCenterXYError=0,peakError=Infinity;
    const ts=new Float32Array(Uint8Array.from(atob(m.triangles),c=>c.charCodeAt(0)).buffer);
    function field(x,y){for(let k=0;k<ts.length;k+=16){const ex=ts[k]-ts[k+4],ey=ts[k+1]-ts[k+5],fx=ts[k+2]-ts[k+4],fy=ts[k+3]-ts[k+5],qx=x-ts[k+4],qy=y-ts[k+5],det=ex*fy-ey*fx,u=(qx*fy-qy*fx)/det,v=(ex*qy-ey*qx)/det;if(Math.min(u,v,1-u-v)<-.00002)continue;const h=ts[k+6]+ts[k+7]*u+ts[k+8]*v+ts[k+9]*u*u+ts[k+10]*u*v+ts[k+11]*v*v+ts[k+12]*u**3+ts[k+13]*u*u*v+ts[k+14]*u*v*v+ts[k+15]*v**3,du=ts[k+7]+2*ts[k+9]*u+ts[k+10]*v+3*ts[k+12]*u*u+2*ts[k+13]*u*v+ts[k+14]*v*v,dv=ts[k+8]+ts[k+10]*u+2*ts[k+11]*v+ts[k+13]*u*u+2*ts[k+14]*u*v+3*ts[k+15]*v*v;return[h,(du*fy-dv*ey)/det,(-du*fx+dv*ex)/det];}throw Error('曲面領域の欠落');}

    for(let i=0;i<g.length/6;i+=Math.max(1,Math.floor(g.length/6/100))){const v=field(g[i*6],g[i*6+2]);maxHeightError=Math.max(maxHeightError,Math.abs(g[i*6+1]-v[0]));maxGradientError=Math.max(maxGradientError,Math.abs(g[i*6+3]+v[1]),Math.abs(g[i*6+5]+v[2]));}
    for(let i=0;i<m.info.controls;i++){maxCenterXYError=Math.max(maxCenterXYError,Math.abs(centers[i*6]-features[i*4]),Math.abs(centers[i*6+2]-features[i*4+1]));if(centers[i*6]===m.info.summit[0]&&centers[i*6+2]===m.info.summit[1])peakError=Math.min(peakError,Math.abs(centers[i*6+1]-m.info.summit[2]));}
    adaptiveEngine.setFairing(40);const filtered=adaptiveEngine.readGeometry();let filterHeightError=0,filterGradientError=0;
    for(let i=0;i<g.length/6;i+=Math.max(1,Math.floor(g.length/6/20))){const x=g[i*6],y=g[i*6+2];if(Math.abs(x)>15000||Math.abs(y)>15000)continue;const expected=field(x,y).map(v=>v*.2);for(const [dx,dy,w] of [[40,0,.1],[-40,0,.1],[0,40,.1],[0,-40,.1],[80,0,.05],[-80,0,.05],[0,80,.05],[0,-80,.05],[40,40,.05],[-40,40,.05],[-40,-40,.05],[40,-40,.05]]){const f=field(x+dx,y+dy);for(let k=0;k<3;k++)expected[k]+=w*f[k];}filterHeightError=Math.max(filterHeightError,Math.abs(filtered[i*6+1]-expected[0]));filterGradientError=Math.max(filterGradientError,Math.abs(filtered[i*6+3]+expected[1]),Math.abs(filtered[i*6+5]+expected[2]));}
    const variation=arr=>{let values=[];const stride=adaptiveEngine.stats.vertices**.5;for(let i=0;i<stride*(stride-1);i++){const a=i*6,b=(i+stride)*6;values.push(Math.hypot(arr[a+3]-arr[b+3],arr[a+5]-arr[b+5]));}values.sort((a,b)=>a-b);return values[Math.floor(values.length*.99)];};const rawVariation=variation(g),filteredVariation=variation(filtered);
    let generations=0;const original=gl.beginTransformFeedback.bind(gl);gl.beginTransformFeedback=(...a)=>{generations++;return original(...a);};for(const pitch of [.15,.5,1.1]){camera.pitch=pitch;draw();}gl.beginTransformFeedback=original;return{filterHeightError,filterGradientError,rawVariation,filteredVariation,maxHeightError,maxGradientError,maxCenterXYError,peakError,generations,error:gl.getError()};
   },region);
   console.log(region,JSON.stringify(check));assert.equal(check.error,0);assert(check.filterHeightError<.02);assert(check.filterGradientError<.01);assert(check.filteredVariation<check.rawVariation);assert.equal(check.generations,0);assert.equal(check.maxCenterXYError,0);assert(check.peakError<.02);assert(check.maxHeightError<.02);assert(check.maxGradientError<.01);checks.push({region,...check});
   await page.locator('#normals').uncheck();assert.equal(await page.evaluate(()=>gl.getError()),0);await page.locator('#normals').check();await page.locator('#splineControlPoints').uncheck();await page.locator('#splineInputPoints').check();await page.waitForFunction(()=>adaptiveEngine.pointStats.inputs>0);assert.equal(await page.evaluate(()=>gl.getError()),0);
   await page.selectOption('#renderer','ridgeSpline');await page.waitForFunction(()=>lpfCurveStatus.renderer==='ridgeSpline');await page.selectOption('#renderer','adaptive');await ready(region,32768);
  }
  assert.deepEqual(errors,[]);fs.writeFileSync('build/adaptive-free-surface-v6/browser-verification.json',JSON.stringify({passed:true,rows,checks,errors},null,2));console.log(JSON.stringify({passed:true,cases:rows.length,checks,errors}));
 }finally{await browser.close();}
})().catch(error=>{console.error(error);process.exit(1);});
