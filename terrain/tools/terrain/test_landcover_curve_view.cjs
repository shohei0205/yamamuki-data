const{chromium}=require('playwright');const assert=require('node:assert/strict');
(async()=>{
 const browser=await chromium.launch({channel:'msedge',headless:true});
 try{
 const page=await browser.newPage({viewport:{width:1500,height:1050}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.goto('http://127.0.0.1:61495/?filter=legacy');
 await page.waitForFunction(()=>window.landcoverPrototype?.currentData?.curveInfo||document.querySelector('#error').textContent,{},{timeout:120000});
 assert.equal(await page.locator('#error').textContent(),'');
 const original=await page.evaluate(()=>({level:landcoverPrototype.currentLevel,info:landcoverPrototype.currentData.curveInfo,error:landcoverPrototype.currentData.areaRelativeError}));
 assert.equal(original.info.method,'shared-quadratic-bezier');assert.ok(original.error<1e-7);
 await page.selectOption('#lod','2');
 await page.selectOption('#mode','curve-split');
 await page.waitForFunction(()=>document.querySelector('#status').textContent.includes('読込完了')&&landcoverPrototype.currentLevel===2,{},{timeout:60000});
 await page.click('#summit');
 // Keep fixed L2 while zooming, so the same original and curved boundaries can be compared.
 await page.waitForTimeout(1000);
 await page.screenshot({path:'build/landcover-vector-fuji-v2/fuji-bezier-comparison.png',fullPage:true});
 await page.selectOption('#curveStrength','1');
 await page.waitForFunction(()=>landcoverPrototype.currentData?.curveInfo?.strength===1||document.querySelector('#error').textContent,{},{timeout:120000});
 assert.equal(await page.locator('#error').textContent(),'');
 await page.selectOption('#curveSteps','8');
 await page.waitForFunction(()=>landcoverPrototype.currentData?.curveInfo?.subdivisions===8||document.querySelector('#error').textContent,{},{timeout:120000});
 assert.equal(await page.locator('#error').textContent(),'');
 assert.deepEqual(errors,[]);
 console.log(JSON.stringify({initial:original,final:await page.evaluate(()=>landcoverPrototype.currentData.curveInfo)},null,2));
 }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exit(1)});
