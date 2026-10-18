// 設定の表示更新と、クリップボードへコピーした全文を確認する。
const assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{const browser=await chromium.launch({headless:true,channel:'msedge'});
try{
 const context=await browser.newContext({viewport:{width:1200,height:800},permissions:['clipboard-read','clipboard-write']});
 const page=await context.newPage(),errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.goto('http://127.0.0.1:61440/heightmap?geometry=polygon9&meshCodec=meshopt-f32-zstd&normalSource=baked&normalCodec=astc4-zstd');
 await page.waitForFunction(()=>!document.getElementById('copySettings').disabled);
 const initial=await page.locator('#settingsLine').inputValue();
 assert.ok(initial.includes('ポリゴン L9'));assert.ok(initial.includes('焼き込み'));assert.ok(initial.includes('1024×1024'));assert.ok(initial.includes('ASTC 4×4'));assert.ok(initial.includes('容量'));assert.ok(initial.includes('視点'));assert.ok(!initial.includes('\n'));
 await page.click('#copySettings');assert.equal(await page.evaluate(()=>navigator.clipboard.readText()),initial);assert.equal(await page.locator('#copyStatus').textContent(),'コピーしました');
 await page.selectOption('#normalResolution','512');await page.waitForFunction(()=>document.getElementById('settingsLine').value.includes('512×512'));
 const changed=await page.locator('#settingsLine').inputValue();assert.notEqual(changed,initial);assert.equal(await page.locator('#copyStatus').textContent(),'');
 await page.locator('#settingsLine').click();assert.equal(await page.locator('#settingsLine').evaluate(e=>e.selectionEnd-e.selectionStart),changed.length);
 await page.selectOption('#normal','face');await page.waitForFunction(()=>document.getElementById('settingsLine').value.includes('陰影 三角形の面'));
 assert.ok(!(await page.locator('#settingsLine').inputValue()).includes('512×512'));
 await page.screenshot({path:'build/terrain-codec-comparison/settings-copy.png'});assert.deepEqual(errors,[]);
 console.log(JSON.stringify({clipboardMatches:true,settingsUpdate:true,clickSelectsAll:true,inactiveSettingsOmitted:true}));
}finally{await browser.close();}})().catch(e=>{console.error(e);process.exitCode=1;});
