'use strict';
const {chromium}=require('playwright'),assert=require('node:assert/strict');
(async()=>{
 const browser=await chromium.launch({channel:'chrome',headless:true,args:['--ignore-gpu-blocklist']});
 try{
  const page=await browser.newPage(),errors=[];page.on('pageerror',e=>errors.push(e.message));
  let release;const gate=new Promise(resolve=>release=resolve);
  await page.route('**/api/ridge?**',async route=>{await gate;await route.continue();});
  await page.route('**/api/mesh-sizes?**',route=>route.fulfill({status:500,body:'simulated failure'}));
  await page.goto('http://127.0.0.1:61470/?region=fuji&quality=coarse');
  await page.waitForFunction(()=>document.getElementById('status').textContent.includes('比較元メッシュを表示中'),{},{timeout:30000});
  assert.equal(await page.evaluate(()=>lpfCurveStatus.error),0);
  assert(await page.evaluate(()=>lpfCurveStatus.vertices>0));
  release();
  await page.waitForFunction(()=>lpfCurveStatus.ridge?.quality==='coarse',{},{timeout:60000});
  await page.waitForFunction(()=>document.getElementById('sizes').textContent.includes('容量を取得できませんでした'));
  assert.equal(await page.evaluate(()=>lpfCurveStatus.error),0);
  await page.unroute('**/api/mesh-sizes?**');await page.selectOption('#level','10');
  await page.waitForFunction(()=>lpfCurveStatus.ridge?.referenceZ===10&&lpfCurveStatus.ridge.referenceMeshBytes>0);
  assert.equal(await page.evaluate(()=>lpfCurveStatus.ridge.referenceMeshBytes),18601);
  assert.deepEqual(errors,[]);
  console.log('PASS: 曲面準備中・容量取得失敗でも地形を保持し、再取得で z10 の容量を表示');
 }finally{await browser.close();}
})().catch(error=>{console.error(error);process.exit(1);});
