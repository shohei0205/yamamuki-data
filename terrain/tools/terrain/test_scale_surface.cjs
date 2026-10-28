'use strict';
const {chromium}=require('playwright'),assert=require('node:assert/strict'),fs=require('node:fs');
(async()=>{const browser=await chromium.launch({channel:'chrome',headless:true,args:['--ignore-gpu-blocklist']});const rows=[],checks=[],errors=[];try{
 const page=await browser.newPage({viewport:{width:1200,height:800}});page.on('pageerror',e=>errors.push(e.message));
 for(const region of ['yari','fuji','tsukuba']){
  await page.goto(`http://127.0.0.1:61473/?region=${region}&renderer=adaptive&points=512&layout=scale&fairing=0`);
  for(const budget of [512,2048,8192,32768]){
   if(budget!==512)await page.selectOption('#adaptiveBudget',String(budget));
   await page.waitForFunction(([r,b])=>window.lpfCurveStatus?.adaptive?.kernel==='Scale-C2-local-quadratic'&&lpfCurveStatus.adaptive.region===r&&lpfCurveStatus.adaptive.budget===b,[region,budget],{timeout:120000});
   const result=await page.evaluate(async({region,budget})=>{
    const m=await(await fetch(`/api/adaptive?region=${region}&budget=${budget}&layout=scale`)).json(),p=new Float32Array(Uint8Array.from(atob(m.triangles),c=>c.charCodeAt(0)).buffer),g=adaptiveEngine.readGeometry(),centers=adaptiveEngine.readControls();
    function field(x,y){let sw=0,sh=0,dwx=0,dwy=0,dnx=0,dny=0;for(let k=0;k<p.length;k+=12){const radius=p[k+2],u=(x-p[k])/radius,v=(y-p[k+1])/radius,r=Math.hypot(u,v);if(r>=1)continue;const s=1-r,w=s**4*(4*r+1),h=p[k+3]+p[k+4]*u+p[k+5]*v+p[k+6]*u*u+p[k+7]*u*v+p[k+8]*v*v,wx=-20*s**3*u/radius,wy=-20*s**3*v/radius,hx=(p[k+4]+2*p[k+6]*u+p[k+7]*v)/radius,hy=(p[k+5]+p[k+7]*u+2*p[k+8]*v)/radius;sw+=w;sh+=w*h;dwx+=wx;dwy+=wy;dnx+=wx*h+w*hx;dny+=wy*h+w*hy;}const h=sh/sw;return[h,(dnx-h*dwx)/sw,(dny-h*dwy)/sw];}
    let maxHeightError=0,maxGradientError=0,markerError=0;
    for(let i=0;i<g.length/6;i+=Math.max(1,Math.floor(g.length/6/80))){const v=field(g[i*6],g[i*6+2]);maxHeightError=Math.max(maxHeightError,Math.abs(g[i*6+1]-v[0]));maxGradientError=Math.max(maxGradientError,Math.abs(g[i*6+3]+v[1]),Math.abs(g[i*6+5]+v[2]));}
    for(let i=0;i<m.info.controls;i++)markerError=Math.max(markerError,Math.abs(centers[i*6]-p[i*12]),Math.abs(centers[i*6+2]-p[i*12+1]));
    let generations=0;const original=gl.beginTransformFeedback.bind(gl);gl.beginTransformFeedback=(...args)=>{generations++;return original(...args);};for(const pitch of [.4,.8,1.1]){camera.pitch=pitch;draw();}gl.beginTransformFeedback=original;
    return{region,budget,kernel:m.info.kernel,storedBytes:m.info.storedBytes,medianSeparation:m.info.medianSeparation,densityRatio:m.info.densityP99/m.info.densityP50,maxHeightError,maxGradientError,markerError,generations,error:gl.getError(),fairing:adaptiveEngine.stats.fairing};
   },{region,budget});
   assert.equal(result.error,0);assert.equal(result.generations,0);assert.equal(result.markerError,0);assert.equal(result.fairing,0);assert(result.maxHeightError<.005);assert(result.maxGradientError<.0001);assert(result.densityRatio<1.5);rows.push(result);console.log(JSON.stringify(result));
   if(budget===512||budget===8192){await page.locator('#splineControlPoints').uncheck();await page.locator('#reset').click();await page.screenshot({path:`build/adaptive-free-surface-v7/${region}-${budget}-overview.png`});}
  }
  await page.selectOption('#adaptiveBudget','512');await page.waitForFunction(()=>lpfCurveStatus.adaptive?.budget===512);await page.selectOption('#adaptiveLayout','legacy');await page.waitForFunction(()=>lpfCurveStatus.adaptive?.kernel==='Thin-plate-global');await page.selectOption('#adaptiveLayout','scale');await page.waitForFunction(()=>lpfCurveStatus.adaptive?.kernel==='Scale-C2-local-quadratic');
  await page.locator('#normals').uncheck();assert.equal(await page.evaluate(()=>gl.getError()),0);await page.locator('#normals').check();checks.push({region,legacySwitch:true,flatNormals:true});
 }
 assert.deepEqual(errors,[]);fs.writeFileSync('build/adaptive-free-surface-v7/browser-verification.json',JSON.stringify({passed:true,rows,checks,errors},null,2));
}finally{await browser.close();}})().catch(e=>{console.error(e);process.exit(1);});
