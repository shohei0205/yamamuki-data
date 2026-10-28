'use strict';
const {chromium}=require('playwright'),assert=require('node:assert/strict');
(async()=>{
 const browser=await chromium.launch({channel:'chrome',headless:true,args:['--ignore-gpu-blocklist']});
 try{
  const page=await browser.newPage({viewport:{width:1200,height:800}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.goto('http://127.0.0.1:61472/?region=yari&fitN=256');
  await page.waitForFunction(()=>window.lpfCurveStatus?.renderer==='ridgeSpline'&&lpfCurveStatus.spline?.region==='yari');
  await page.locator('#ridgeFocus').click();
  const pixels=()=>page.evaluate(()=>{draw();const p=new Uint8Array(gl.drawingBufferWidth*gl.drawingBufferHeight*4);gl.readPixels(0,0,gl.drawingBufferWidth,gl.drawingBufferHeight,gl.RGBA,gl.UNSIGNED_BYTE,p);let blue=0,orange=0;for(let i=0;i<p.length;i+=4){if(p[i]<80&&p[i+1]>160&&p[i+2]>220)blue++;if(p[i]>220&&p[i+1]>40&&p[i+1]<140&&p[i+2]<70)orange++;}return{blue,orange,error:gl.getError(),...splineEngine.pointStats};});
  let p=await pixels();assert.equal(p.error,0);assert.equal(p.controls,66049);assert(p.blue>0);
  await page.locator('#splineControlPoints').uncheck();p=await pixels();assert.equal(p.blue,0);
  await page.locator('#splineInputPoints').check();await page.waitForFunction(()=>splineEngine.pointStats.inputs===66203);p=await pixels();assert(p.orange>0);assert.equal(p.error,0);
  await page.locator('#splineControlPoints').check();await page.screenshot({path:'build/ridge-spline-prototype-v4/control-points.png'});
  await page.locator('#splinePointXray').uncheck();assert.equal((await pixels()).error,0);
  await page.selectOption('#region','fuji');await page.waitForFunction(()=>window.lpfCurveStatus?.spline?.region==='fuji'&&splineEngine.pointStats.inputs===12478);
  await page.selectOption('#ridgeSplineQuality','128');await page.waitForFunction(()=>lpfCurveStatus.spline?.n===128&&splineEngine.pointStats.controls===16641);
  assert.equal((await pixels()).error,0);assert.deepEqual(errors,[]);console.log('PASS: control/input points, pixel colors, visibility, depth, region and interval changes');
 }finally{await browser.close();}
})().catch(error=>{console.error(error);process.exit(1);});
