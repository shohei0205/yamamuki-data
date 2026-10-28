// 比較ページの3予算、左右切替、回転操作を確認する。
const{chromium}=require('playwright'),assert=require('node:assert/strict');
(async()=>{const browser=await chromium.launch({channel:'chrome',headless:true});try{
 const page=await browser.newPage({viewport:{width:1400,height:900}}),errors=[];page.on('pageerror',e=>errors.push(e.message));await page.goto('http://127.0.0.1:61485/loop-fit?budget=2048');await page.waitForFunction(()=>window.previewReady);await page.waitForTimeout(100);
 assert.equal(await page.evaluate(()=>previewError),0);assert.ok(await page.evaluate(()=>fitReport.holdoutAfter.rmse<fitReport.holdoutBefore.rmse));await page.screenshot({path:'build/loop-dem-fit-v1/compare-2048.png'});
 for(const budget of ['512','8192']){await page.selectOption('#budget',budget);await page.waitForFunction(b=>window.fitReport?.controlVertices===Number(b),budget);await page.waitForTimeout(100);assert.equal(await page.evaluate(()=>previewError),0);}
 await page.uncheck('#compare');await page.mouse.move(800,600);await page.mouse.down({button:'right'});await page.mouse.move(850,620,{steps:10});await page.mouse.up({button:'right'});await page.waitForTimeout(100);assert.equal(await page.evaluate(()=>previewError),0);await page.screenshot({path:'build/loop-dem-fit-v1/fit-8192.png'});assert.deepEqual(errors,[]);console.log('Loop比較・3予算・左右切替・回転・WebGL：成功');
 }finally{await browser.close()}})().catch(e=>{console.error(e);process.exitCode=1});
