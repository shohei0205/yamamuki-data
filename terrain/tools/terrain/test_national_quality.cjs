// 表示レベルで実際の法線・陰影解像度と描画結果が変わることを確認する。
const assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{const browser=await chromium.launch({headless:true,channel:'msedge'});try{
 const page=await browser.newPage({viewport:{width:1300,height:1100}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.goto('http://127.0.0.1:61440/national-shading');await page.waitForFunction(()=>window.nationalShadingStatus);
 const differences={};
 for(const [mode,button]of [['normal','#normalMode'],['shade','#shadeMode']]){
  await page.click(button);await page.selectOption('#normalResolution','equal');
  await page.selectOption('#geometryLevel','4');await page.waitForFunction(()=>window.nationalShadingStatus.geometryLevel===4&&!document.getElementById('geometryLevel').disabled);
  const baseline=await page.evaluate(()=>window.nationalShadingStatus);assert.equal(baseline.textureWidth,448);
  await page.evaluate(()=>{draw();window.lodPixels=new Uint8Array(canvas.width*canvas.height*4);gl.readPixels(0,0,canvas.width,canvas.height,gl.RGBA,gl.UNSIGNED_BYTE,lodPixels);});
  await page.screenshot({path:`build/terrain-national-shading/preview-${mode}-L4.png`});
  for(const level of [5,6,7]){
   await page.selectOption('#geometryLevel',String(level));await page.waitForFunction(l=>window.nationalShadingStatus.geometryLevel===l&&!document.getElementById('geometryLevel').disabled,level);
   const state=await page.evaluate(()=>window.nationalShadingStatus);assert.equal(state.normalLevel,level);assert.equal(state.shadeLevel,level);assert.equal(state.textureWidth,448*2**(level-4));assert.equal(state.glError,0);
   for(const key of ['center','scale','bearing','tilt','sun','strength','contrast'])assert.deepEqual(state[key],baseline[key]);
  }
  differences[mode]=await page.evaluate(()=>{draw();const p=new Uint8Array(lodPixels.length);gl.readPixels(0,0,canvas.width,canvas.height,gl.RGBA,gl.UNSIGNED_BYTE,p);let n=0;for(let i=0;i<p.length;i++)n+=Math.abs(p[i]-lodPixels[i]);return n/p.length;});assert.ok(differences[mode]>.01);
  await page.screenshot({path:`build/terrain-national-shading/preview-${mode}-L7.png`});
 }
 await page.locator('#sunAzimuth').fill('315');await page.locator('#sunAltitude').fill('45');await page.click('#sunBake');await page.waitForFunction(()=>!document.getElementById('sunBake').disabled);
 assert.equal(await page.evaluate(()=>window.nationalShadingStatus.shadeLevel),7);
 assert.deepEqual(errors,[]);console.log(JSON.stringify({differences,errors}));
}finally{await browser.close()}})().catch(e=>{console.error(e);process.exit(1)});
