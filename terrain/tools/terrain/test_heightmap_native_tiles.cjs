// 各レベルが実タイル 2×2 になり、圧縮結果にも複数タイルが含まれることを確認する。
const assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{const browser=await chromium.launch({headless:true,channel:'msedge'});try{
 const page=await browser.newPage({viewport:{width:1300,height:1100}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.goto('http://127.0.0.1:61440/heightmap?tiles=2&level=10');await page.waitForFunction(()=>window.heightmapStatus?.level===10);
 for(const level of [4,5,6,7,8,9,10,11,12,13,14]){
  await page.selectOption('#dataLevel',String(level));await page.waitForFunction(l=>window.heightmapStatus?.level===l&&!document.getElementById('dataLevel').disabled,level);
  const state=await page.evaluate(()=>window.heightmapStatus);assert.equal(state.nativeTiles,true);assert.equal(state.heightTiles,4);assert.ok(state.normalTiles>=4);assert.equal(state.glError,0);
  assert.equal(await page.evaluate(()=>window.heightmapMetadata.width),512);
 }
 await page.selectOption('#tileLines','3');await page.fill('#jxlNormalDistance','0');await page.click('#jxlApply');await page.waitForFunction(()=>dynamicJXL?.tileGrid===2&&!document.getElementById('jxlApply').disabled);
 const result=await page.evaluate(()=>dynamicJXL);assert.equal(result.heightTileColumns,2);assert.equal(result.heightTileRows,2);assert.equal(result.normalTileColumns,2);assert.equal(result.normalTileRows,2);assert.equal(result.maxHeightError,0);assert.equal(result.normalMax,0);
 await page.screenshot({path:'build/terrain-jxl-dynamic/preview-native-tiles.png'});
 assert.deepEqual(errors,[]);console.log('全レベル実タイル 2×2・境界線・可逆圧縮の複数タイル: OK');
}finally{await browser.close()}})().catch(e=>{console.error(e);process.exit(1)});
