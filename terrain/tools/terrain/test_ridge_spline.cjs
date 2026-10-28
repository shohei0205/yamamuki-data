'use strict';
const {chromium}=require('playwright'),assert=require('node:assert/strict'),fs=require('node:fs');
(async()=>{
 const browser=await chromium.launch({channel:'chrome',headless:true,args:['--ignore-gpu-blocklist']});
 const output='build/ridge-spline-prototype-v4';
 try{
  const page=await browser.newPage({viewport:{width:1200,height:800}}),errors=[],rows=[];
  page.on('pageerror',e=>errors.push(e.message));
  const ready=async(region,n)=>{await page.waitForFunction(([r,n])=>window.lpfCurveStatus?.renderer==='ridgeSpline'&&lpfCurveStatus.spline?.region===r&&lpfCurveStatus.spline.n===n,[region,n],{timeout:60000});const v=await page.evaluate(()=>lpfCurveStatus);assert.equal(v.error,0);assert(v.spline.peakError<.01);return v;};
  for(const region of ['yari','fuji','tsukuba']){
   await page.goto(`http://127.0.0.1:61472/?region=${region}&fitN=128`);rows.push(await ready(region,128));
   for(const n of [256,512]){await page.selectOption('#ridgeSplineQuality',String(n));rows.push(await ready(region,n));}
   await page.locator('#ridgeFocus').click();await page.screenshot({path:`${output}/${region}-spline.png`});
   await page.locator('#normals').uncheck();assert.equal((await page.evaluate(()=>lpfCurveStatus)).error,0);await page.locator('#normals').check();
   await page.selectOption('#renderer','bspline');await page.waitForFunction(()=>window.lpfCurveStatus?.renderer==='bspline'&&!lpfCurveStatus.spline?.fitSource);
   await page.selectOption('#renderer','ridgeSpline');await ready(region,512);
   await page.selectOption('#level','10');await page.waitForFunction(()=>lpfCurveStatus.z===10&&lpfCurveStatus.renderer==='ridgeSpline');
  }
  assert.deepEqual(errors,[]);fs.writeFileSync(`${output}/browser-verification.json`,JSON.stringify({passed:true,rows,errors},null,2));console.log(JSON.stringify({passed:true,cases:rows.length,errors}));
 }finally{await browser.close();}
})().catch(error=>{console.error(error);process.exit(1);});
