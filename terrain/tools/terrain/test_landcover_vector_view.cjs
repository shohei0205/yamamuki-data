const {chromium}=require('playwright');
const assert=require('node:assert/strict');
(async()=>{
 const browser=await chromium.launch({channel:'msedge',headless:true});
 try{
  const page=await browser.newPage({viewport:{width:1500,height:1050}}),errors=[];
  page.on('pageerror',e=>errors.push(e.message));
  await page.goto('http://127.0.0.1:61495/?filter=legacy');
  await page.waitForFunction(()=>window.landcoverPrototype?.currentLevel>=0,{},{timeout:240000});
  await page.selectOption('#curve','line');
  for(const n of [0,1,2,3,4]){
   await page.selectOption('#lod',String(n));
   await page.waitForFunction(n=>window.landcoverPrototype.currentLevel===n,n,{timeout:60000});
   assert.ok(await page.evaluate(n=>landcoverPrototype.currentData.areaRelativeError<1e-7,n));
  }
  await page.selectOption('#lod','auto');
  await page.click('#summit');
  await page.waitForTimeout(1000);
  await page.screenshot({path:'build/landcover-vector-fuji-v2/fuji-detail.png',fullPage:true});
  await page.click('#reset');
  await page.selectOption('#mode','split');
  await page.waitForTimeout(1000);
  await page.screenshot({path:'build/landcover-vector-fuji-v2/fuji-comparison.png',fullPage:true});
  const report=await page.evaluate(()=>({levels:landcoverPrototype.manifest.levels.length,current:landcoverPrototype.currentLevel,drawMs:landcoverPrototype.drawMs,stats:document.querySelector('#stats').textContent}));
  assert.equal(report.levels,5);assert.deepEqual(errors,[]);
  console.log(JSON.stringify(report,null,2));
 }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exit(1)});
