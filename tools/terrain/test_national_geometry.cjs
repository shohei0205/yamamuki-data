// 全地形レベルへの切替と、画角・太陽・陰影方式の保持を確認する。
const assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{const browser=await chromium.launch({headless:true,channel:'msedge'});try{
 const page=await browser.newPage({viewport:{width:1300,height:1100}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.goto('http://127.0.0.1:61440/national-shading');await page.waitForFunction(()=>window.nationalShadingStatus);
 await page.locator('#sunAzimuth').fill('90');await page.locator('#sunAltitude').fill('30');await page.locator('#tilt').fill('40');await page.click('#shadeMode');
 const keys=['center','scale','bearing','tilt','strength','contrast','mode','resolution','sun','bakedSun'];
 const view=await page.evaluate(keys=>Object.fromEntries(keys.map(k=>[k,window.nationalShadingStatus[k]])),keys),results=[];
 let previous=0;
 for(const level of [4,5,6,7,4]){
  if(level!==4||previous!==0){await page.selectOption('#geometryLevel',String(level));await page.waitForFunction(l=>window.nationalShadingStatus.geometryLevel===l&&!document.getElementById('geometryLevel').disabled,level);}
  const state=await page.evaluate(()=>window.nationalShadingStatus);
  assert.equal(state.geometryLevel,level);assert.equal(state.geometryWidth,448*2**(level-4));assert.equal(state.glError,0);
  for(const key of keys)assert.deepEqual(state[key],view[key]);
  if(previous&&level>4)assert.ok(state.triangles>previous);previous=state.triangles;results.push({level,triangles:state.triangles,transferBytes:state.geometryTransferBytes});
 }
 assert.deepEqual(errors,[]);console.log(JSON.stringify(results));
}finally{await browser.close()}})().catch(e=>{console.error(e);process.exit(1)});
