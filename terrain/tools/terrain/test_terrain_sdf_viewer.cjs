// 距離場の3解像度・左右比較・操作・PNG表示を確認する。
const {chromium}=require('playwright'),assert=require('node:assert/strict');
(async()=>{const browser=await chromium.launch({channel:'chrome',headless:true});try{
 const page=await browser.newPage({viewport:{width:1400,height:900}}),errors=[];page.on('pageerror',e=>errors.push(e.message));await page.goto('http://127.0.0.1:61485/sdf-fit?size=128');await page.waitForFunction(()=>window.previewReady);await page.waitForTimeout(100);assert.equal(await page.evaluate(()=>previewError),0);await page.screenshot({path:'build/terrain-sdf-v1/compare-128.png'});
 for(const size of ['64','256']){await page.selectOption('#budget',size);await page.waitForFunction(n=>window.fitReport?.horizontal===Number(n),size);await page.waitForTimeout(100);assert.equal(await page.evaluate(()=>previewError),0);}
 await page.uncheck('#compare');await page.mouse.move(800,600);await page.mouse.down({button:'right'});await page.mouse.move(850,620,{steps:10});await page.mouse.up({button:'right'});await page.waitForTimeout(100);await page.screenshot({path:'build/terrain-sdf-v1/field-256.png'});assert.equal(await page.evaluate(()=>previewError),0);assert.deepEqual(errors,[]);console.log('SDF表示・3解像度・回転・WebGL：成功');
 }finally{await browser.close()}})().catch(e=>{console.error(e);process.exitCode=1});
