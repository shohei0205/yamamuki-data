'use strict';
const{chromium}=require('playwright'),fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict');
(async()=>{
 const output=path.resolve('build/feature-surface-prototype-v2/browser-check');fs.mkdirSync(output,{recursive:true});
 const browser=await chromium.launch({channel:'chrome',headless:true,args:['--ignore-gpu-blocklist']}),page=await browser.newPage({viewport:{width:1300,height:850}}),errors=[],rows=[];
 page.on('pageerror',e=>errors.push(e.message));
 const ready=async(region,budget)=>{await page.waitForFunction(([r,b])=>window.lpfCurveStatus?.features?.region===r&&lpfCurveStatus.features.budget===b,[region,budget],{timeout:120000});const s=await page.evaluate(()=>lpfCurveStatus);assert.equal(s.error,0);assert.equal(s.renderer,'features');assert(s.surfaceTriangles>0);return s;};
 try{
  for(const region of ['fuji','yari','tsukuba']){
   await page.goto(`http://127.0.0.1:61468/?region=${region}&budget=8`);rows.push(await ready(region,8));
   for(const budget of [32,128,512]){await page.selectOption('#featureCount',String(budget));rows.push(await ready(region,budget));}
   await page.screenshot({path:path.join(output,`${region}-512.png`)});
   const geometry=await page.evaluate(async region=>{
    const model=await(await fetch(`/api/features?region=${region}&budget=512`)).json(),points=featureEngine.readGeometry();let maxHeightError=0,maxGradientError=0;
    for(let i=0;i<points.length/6;i+=Math.max(1,Math.floor(points.length/6/80))){const x=points[i*6],y=points[i*6+2];let h=model.base[0]+(model.base[1]*x+model.base[2]*y)/model.span,gx=model.base[1]/model.span,gy=model.base[2]/model.span;for(const p of model.features){const dx=x-p[0],dy=y-p[1],cs=Math.cos(p[4]),sn=Math.sin(p[4]),u=(dx*cs+dy*sn)/p[2],v=(-dx*sn+dy*cs)/p[3],g=p[5]*Math.exp(-.5*(u*u+v*v));h+=g;gx+=g*(-u*cs/p[2]+v*sn/p[3]);gy+=g*(-u*sn/p[2]-v*cs/p[3]);}maxHeightError=Math.max(maxHeightError,Math.abs(h-points[i*6+1]));maxGradientError=Math.max(maxGradientError,Math.abs(-gx-points[i*6+3]),Math.abs(-gy-points[i*6+5]));}
    let generations=0;const original=gl.beginTransformFeedback.bind(gl);gl.beginTransformFeedback=(...args)=>{generations++;return original(...args);};for(const pitch of [.1,.5,1.1]){camera.pitch=pitch;camera.yaw+=.1;draw();}gl.beginTransformFeedback=original;return {maxHeightError,maxGradientError,generations,error:gl.getError()};
   },region);
   assert(geometry.maxHeightError<.1,JSON.stringify(geometry));assert(geometry.maxGradientError<.001);assert.equal(geometry.generations,0);assert.equal(geometry.error,0);
   fs.writeFileSync(path.join(output,`${region}-geometry.json`),JSON.stringify(geometry,null,2));
   await page.selectOption('#featureCount','128');await ready(region,128);await page.locator('#featureCenters').check();await page.screenshot({path:path.join(output,`${region}-128-centers.png`)});await page.locator('#featureCenters').uncheck();
   for(const division of [1,2,4]){await page.selectOption('#divisions',String(division));assert.equal((await ready(region,128)).surfaceTriangles,2*(64*division)**2);}
   await page.locator('#normals').uncheck();assert.equal((await ready(region,128)).error,0);await page.locator('#normals').check();await page.locator('#wire').check();assert.equal((await ready(region,128)).error,0);await page.locator('#wire').uncheck();
   await page.evaluate(()=>{camera.pitch=.15;camera.distance=20000;draw();});await page.screenshot({path:path.join(output,`${region}-low-view.png`)});
   await page.selectOption('#renderer','bspline');assert.equal((await page.evaluate(()=>lpfCurveStatus)).renderer,'bspline');await page.selectOption('#renderer','features');assert.equal((await ready(region,128)).renderer,'features');
  }
  assert.deepEqual(errors,[]);fs.writeFileSync(path.join(output,'verification.json'),JSON.stringify({passed:true,rows,errors},null,2));console.log(JSON.stringify({passed:true,cases:rows.length,errors}));
 }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exit(1);});
