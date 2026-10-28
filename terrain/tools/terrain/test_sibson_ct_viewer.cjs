// 同じ制御点による補間切り替えと、軽量描画経路を確認する。
const {chromium}=require('playwright'),assert=require('node:assert/strict');
(async()=>{const browser=await chromium.launch({channel:'chrome',headless:true});try{
 const page=await browser.newPage({viewport:{width:1200,height:800}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.goto('http://127.0.0.1:61485/?region=yari&method=ct&level=100&span=16066&draw=legacy&shade=vertex');
 async function ready(){await page.waitForFunction(()=>window.previewReady&&!building&&!dirty&&!inflight&&resident.size===targets.length,{},{timeout:120000});}
 await ready();assert.equal(await page.locator('#shade').isDisabled(),true);assert.equal(await page.evaluate(()=>[...resident.values()].every(t=>t.info.method==='ct')),true);
 await page.screenshot({path:'build/sibson-region128-v1/ct-yari.png'});
 await page.selectOption('#interpolation','sibson');await ready();assert.equal(await page.evaluate(()=>[...resident.values()].every(t=>t.info.method==='sibson')),true);
 await page.selectOption('#interpolation','ct');await ready();assert.equal(await page.evaluate(()=>[...resident.values()].every(t=>t.info.method==='ct')),true);assert.equal(await page.evaluate(()=>previewError),0);assert.deepEqual(errors,[]);console.log('Clough–Tocher表示・往復切り替え：成功');
 }finally{await browser.close()}})().catch(e=>{console.error(e);process.exitCode=1});
