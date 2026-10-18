// 太陽の変更による法線の即時反映と、実際に再生成した縮小陰影の反映を確認する。
const assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{const browser=await chromium.launch({headless:true,channel:'msedge'});try{
 const page=await browser.newPage({viewport:{width:1300,height:1100}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.goto('http://127.0.0.1:61440/national-shading');await page.waitForFunction(()=>window.nationalShadingStatus);
 const camera=await page.evaluate(()=>({center:[...center],scale,bearing}));
 await page.evaluate(()=>{draw();window.sunPixels=new Uint8Array(canvas.width*canvas.height*4);gl.readPixels(0,0,canvas.width,canvas.height,gl.RGBA,gl.UNSIGNED_BYTE,sunPixels);});
 await page.locator('#sunAzimuth').fill('90');await page.locator('#sunAltitude').fill('30');
 assert.deepEqual(await page.evaluate(()=>window.nationalShadingStatus.sun),{azimuth:90,altitude:30});
 const diff=await page.evaluate(()=>{draw();const p=new Uint8Array(sunPixels.length);gl.readPixels(0,0,canvas.width,canvas.height,gl.RGBA,gl.UNSIGNED_BYTE,p);let n=0;for(let i=0;i<p.length;i++)n+=Math.abs(p[i]-sunPixels[i]);return n/p.length;});assert.ok(diff>.01);
 assert.ok((await page.locator('#sunStatus').textContent()).includes('旧光源'));
 await page.click('#sunBake');
 await page.waitForFunction(()=>window.nationalShadingStatus?.bakedSun.azimuth===90&&window.nationalShadingStatus?.bakedSun.altitude===30&&!document.getElementById('sunBake').disabled,null,{timeout:300000});
 assert.deepEqual(await page.evaluate(()=>({center:[...center],scale,bearing})),camera);
 await page.click('#shadeMode');assert.deepEqual(await page.evaluate(()=>window.nationalShadingStatus.sun),await page.evaluate(()=>window.nationalShadingStatus.bakedSun));
 await page.screenshot({path:'build/terrain-national-shading/sun-east30.png'});
 await page.locator('#sunAzimuth').fill('315');await page.locator('#sunAltitude').fill('45');await page.click('#sunBake');
 await page.waitForFunction(()=>window.nationalShadingStatus.bakedSun.azimuth===315&&!document.getElementById('sunBake').disabled);
 assert.equal(await page.evaluate(()=>window.nationalShadingStatus.glError),0);assert.deepEqual(errors,[]);
 console.log(JSON.stringify({sunPixelDifference:diff,cachedDefault:true,cameraPreserved:true,errors}));
}finally{await browser.close()}})().catch(e=>{console.error(e);process.exit(1)});
