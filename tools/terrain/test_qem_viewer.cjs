// 最高詳細から削減した各 LOD の表示と比較操作を確かめる。
const assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{
 const browser=await chromium.launch({headless:true,channel:'msedge'});
 try{
  const page=await browser.newPage({viewport:{width:1400,height:900}}),errors=[],requests=[];
  page.on('pageerror',e=>errors.push(e.message));page.on('request',r=>requests.push(r.url()));
  await page.goto('http://127.0.0.1:61434/qem-preview?z=11&fullscreen=1');
  await page.waitForFunction(()=>document.getElementById('stats').textContent.includes('三角形'));
  assert.equal(await page.locator('#terrainLevel').isEnabled(),true);
  assert.equal(await page.evaluate(()=>data.qemComparison.triangles),10530);
  assert.equal(await page.evaluate(()=>document.getElementById('method').value),'shaded');
  assert.equal(await page.evaluate(()=>gl.getError()),0);
  await page.screenshot({path:'build/terrain-qem-final/yari-level11.png'});
  await page.selectOption('#qemSurface','wireframe');
  await page.screenshot({path:'build/terrain-qem-final/yari-level11-triangles.png'});
  await page.locator('#bearing').evaluate(e=>{e.value='210';e.dispatchEvent(new Event('input'));});
  await page.evaluate(()=>{viewPan[0]=.15;viewPan[1]=-.1;draw();});
  await page.selectOption('#terrainLevel','8');
  await page.waitForFunction(()=>data.coverage.zoom===8&&document.getElementById('stats').textContent.includes('三角形'));
  assert.equal(await page.locator('#bearing').inputValue(),'210');
  assert.deepEqual(await page.evaluate(()=>viewPan),[.15,-.1]);
  assert.equal(await page.locator('#qemSurface').inputValue(),'wireframe');
  await page.selectOption('#qemSurface','shaded');
  assert.equal(await page.evaluate(()=>document.getElementById('method').value),'shaded');
  await page.screenshot({path:'build/terrain-qem-final/yari-level8.png'});
  await page.selectOption('#terrainLevel','14');
  await page.waitForFunction(()=>data.coverage.zoom===14&&document.getElementById('stats').textContent.includes('三角形'));
  await page.screenshot({path:'build/terrain-qem-final/yari-level14.png'});
  await page.selectOption('#terrainLevel','reference');
  await page.waitForFunction(()=>data.qemComparison.kind==='reference'&&document.getElementById('stats').textContent.includes('三角形'));
  await page.screenshot({path:'build/terrain-qem-final/yari-reference.png'});
  assert.equal(await page.evaluate(()=>gl.getError()),0);
  assert.equal(requests.some(url=>url.includes('/sqlite/')),false);
  assert.deepEqual(errors,[]);
  console.log(JSON.stringify({levels:[11,8,14],reference:true,cameraPreserved:true,serverGeometryRequests:0,errors}));
 }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
