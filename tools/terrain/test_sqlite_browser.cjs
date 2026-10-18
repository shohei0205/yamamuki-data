// SQLite の中継だけで、待機表示・地形・追加レイヤー・縮尺変更が動くことを確認する。
const assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{
 const browser=await chromium.launch({headless:true,channel:'msedge'});
 try{
  const page=await browser.newPage({viewport:{width:1200,height:800}}),errors=[],urls=[];
  page.on('pageerror',e=>errors.push(e.message));page.on('request',r=>urls.push(r.url()));
  let release,arrived,hold=true;const gate=new Promise(resolve=>release=resolve),arrival=new Promise(resolve=>arrived=resolve);
  await page.route('**/sqlite/mesh?**',async route=>{
   if(hold){hold=false;const response=await route.fetch();arrived();await gate;await route.fulfill({response});}else await route.continue();
  });
  const start=Date.now();await page.goto('http://127.0.0.1:61434/terrain?lat=36.32155&lon=137.6476&size=30&fullscreen=1&levelMode=auto');
  await arrival;
  const before=await page.locator('#dataSize').textContent();assert.match(before,/SQLite 応答待ち/);
  await page.waitForTimeout(400);assert.notEqual(await page.locator('#dataSize').textContent(),before);release();
  await page.waitForFunction(()=>document.getElementById('stats').textContent.includes('三角形'),null,{timeout:120000});
  const firstTerrainMs=Date.now()-start;
  assert.equal(await page.evaluate(()=>data.transferSize.source),'sqlite');assert.equal(await page.evaluate(()=>data.indices instanceof Uint32Array),true);
  await page.waitForFunction(()=>data.overlays?.nationalLines&&!lodBusy,null,{timeout:120000});
  const initial=await page.evaluate(()=>({size:data.transferSize,total:transferStats.total,longitude:data.coverage.longitude,gl:gl.getError()}));
  assert.ok(initial.size.compressed>0);assert.ok(initial.size.decoded>initial.size.compressed);assert.equal(initial.gl,0);
  await page.screenshot({path:'build/terrain-national/sqlite-relay-yari.png'});
  await page.selectOption('#terrainLevel','10');
  await page.waitForFunction(()=>data.coverage.levelMode==='manual'&&data.coverage.zoom===10&&!lodBusy,null,{timeout:120000});
  await page.waitForFunction(()=>data.overlays?.nationalLines,null,{timeout:120000});
  assert.match(await page.locator('#dataSize').textContent(),/保存形式/);
  await page.locator('#terrainLayer').uncheck();assert.equal(await page.evaluate(()=>gl.getError()),0);await page.locator('#terrainLayer').check();
  await page.selectOption('#terrainLevel','auto');
  await page.waitForFunction(()=>data.coverage.levelMode==='auto'&&!lodBusy,null,{timeout:120000});
  assert.equal(urls.some(url=>new URL(url).pathname==='/terrain-data'),false);
  assert.deepEqual(errors,[]);assert.equal(await page.evaluate(()=>gl.getError()),0);
  console.log(JSON.stringify({firstTerrainMs,initial,requestCount:urls.filter(url=>url.includes('/sqlite/')).length,errorCount:errors.length}));
 }finally{await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
