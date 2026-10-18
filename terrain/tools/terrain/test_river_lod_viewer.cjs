// 地形を非表示・手動固定したまま、河川だけが拡大率で切り替わることを確認する。
const assert=require('node:assert/strict');
const {chromium}=require('playwright');
(async()=>{const browser=await chromium.launch({headless:true,channel:'msedge'});
try{
 const page=await browser.newPage({viewport:{width:1000,height:800}});const errors=[];
 page.on('pageerror',e=>errors.push(e.message));
 await page.addInitScript(()=>{localStorage.setItem('terrain-preview-occludeLayers','false');localStorage.setItem('terrain-preview-terrainLayer','false');});
 console.log('初期表示を読み込み');await page.goto('http://127.0.0.1:61434/terrain?lat=35.4&lon=138.73&size=220&levelMode=manual&z=8&fullscreen=1',{timeout:60000});
 await page.waitForFunction(()=>data.overlays?.riverLevel===10&&!lodBusy,null,{timeout:60000});
 assert.ok(await page.evaluate(()=>data.overlays.layers.water.lines.length>0));
 await page.evaluate(()=>{for(const id of ['terrainLayer','trailLayer','boundaryLayer','occludeLayers'])document.getElementById(id).checked=false;document.getElementById('waterLayer').checked=true;draw();});
 async function zoom(size,river){console.log('表示範囲',size,'河川',river);await page.evaluate(size=>{const el=document.getElementById('zoom');el.value=baseSpan/1000*Math.max(1,canvas.clientWidth/canvas.clientHeight)*1.5/size;el.dispatchEvent(new Event('input'));},size);await page.waitForFunction(river=>data.overlays.riverLevel===river&&!lodBusy,river,{timeout:60000});assert.equal(await page.evaluate(()=>data.coverage.zoom),8);assert.equal(await page.isChecked('#terrainLayer'),false);assert.ok(await page.evaluate(()=>data.overlays.layers.water.lines.length>0));}
 await zoom(200,10);
 await zoom(30,12);
 const detail=await page.evaluate(()=>data.overlays.layers.water.lines.length);
 await page.screenshot({path:'build/terrain-national/fuji-river-lod-water-only.png'});
 await zoom(600,8);
 assert.equal(await page.evaluate(()=>data.coverage.zoom),8);
 assert.ok((await page.textContent('#lodStatus')).includes('河川 8'));
 assert.equal(await page.evaluate(()=>gl.getError()),0);assert.deepEqual(errors,[]);
 console.log(JSON.stringify({terrainLevel:8,detailedLineValues:detail,wideRiverLevel:await page.evaluate(()=>data.overlays.riverLevel),errors}));
}finally{await browser.close();}})().catch(e=>{console.error(e);process.exitCode=1;});
