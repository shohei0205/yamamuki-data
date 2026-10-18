// 追加レイヤーの応答が止まっていても、別の縮尺の地形を表示できる。
const assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{const browser=await chromium.launch({headless:true,channel:'msedge'});let release;
 try{
  const page=await browser.newPage({viewport:{width:1000,height:800}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
  let first=true,arrived;const gate=new Promise(resolve=>release=resolve),arrival=new Promise(resolve=>arrived=resolve);
  await page.route('**/sqlite/lines?**',async route=>{
   if(first){first=false;arrived();await gate;try{await route.continue();}catch{}}else await route.continue();
  });
  await page.goto('http://127.0.0.1:61434/terrain?lat=36.32155&lon=137.6476&size=30&fullscreen=1&levelMode=auto');
  await arrival;assert.equal(await page.evaluate(()=>!!data.overlays),false);
  await page.selectOption('#terrainLevel','10');
  await page.waitForFunction(()=>data.coverage.levelMode==='manual'&&data.coverage.zoom===10&&!lodBusy,null,{timeout:60000});
  assert.equal(await page.evaluate(()=>gl.getError()),0);release();
  await page.waitForFunction(()=>data.overlays?.nationalLines,null,{timeout:60000});
  assert.equal(await page.evaluate(()=>data.coverage.zoom),10);assert.deepEqual(errors,[]);
  console.log('追加レイヤーの待機中にも縮尺 10 の地形を表示し、古いレイヤーを中断して新しいレイヤーへ切り替えました。');
 }finally{release?.();await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
