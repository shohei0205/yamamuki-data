// 地域の切替、設定の引継ぎ、筑波山の動的 JPEG XL を確認する。
const assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{const browser=await chromium.launch({headless:true,channel:'msedge'});try{
 const page=await browser.newPage({viewport:{width:1300,height:1000}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.goto('http://127.0.0.1:61440/heightmap?level=11');await page.waitForFunction(()=>window.heightmapStatus?.level===11);
 await page.selectOption('#normalResolution','256');await page.selectOption('#normal','gpu');await page.fill('#jxlNormalDistance','0.5');await page.locator('#light').fill('120');
 await page.click('#regionTsukuba');await page.waitForFunction(()=>window.heightmapStatus?.level===11&&document.getElementById('regionTsukuba').disabled);
 assert.ok((await page.locator('#regionTitle').textContent()).includes('筑波山'));assert.equal(await page.locator('#normal').inputValue(),'gpu');assert.equal(await page.locator('#normalResolution').inputValue(),'256');assert.equal(await page.locator('#light').inputValue(),'120');assert.equal(await page.locator('#jxlNormalDistance').inputValue(),'0.5');
 assert.equal(await page.evaluate(()=>window.heightmapMetadata.west),14560);
 await page.click('#jxlApply');await page.waitForFunction(()=>dynamicJXL?.region==='tsukuba'&&!document.getElementById('jxlApply').disabled);
 assert.equal(await page.evaluate(()=>window.heightmapStatus.normal),'gpu');assert.equal(await page.evaluate(()=>dynamicJXL.maxHeightError),0);assert.equal(await page.evaluate(()=>window.heightmapStatus.glError),0);
 await page.screenshot({path:'build/terrain-jxl-dynamic/tsukuba.png'});
 await page.click('#regionYari');await page.waitForFunction(()=>window.heightmapStatus?.level===11&&document.getElementById('regionYari').disabled);
 assert.equal(await page.evaluate(()=>window.heightmapMetadata.west),14448);assert.deepEqual(errors,[]);
 console.log('槍ヶ岳／筑波山の切替・設定保持・地域別圧縮: OK');
}finally{await browser.close()}})().catch(e=>{console.error(e);process.exit(1)});
