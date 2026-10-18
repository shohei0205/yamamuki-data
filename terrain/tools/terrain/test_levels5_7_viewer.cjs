// 5・7 の手動固定と、地形・全国の線の自動切り替えを実データで確認する。
const assert=require('node:assert/strict');
const {chromium}=require('playwright');
(async()=>{const browser=await chromium.launch({headless:true,channel:'msedge'});
try{const page=await browser.newPage({viewport:{width:1000,height:800}});const errors=[];
page.on('pageerror',e=>errors.push(e.message));await page.addInitScript(()=>localStorage.setItem('terrain-preview-occludeLayers','false'));
console.log('初期表示');await page.goto('http://127.0.0.1:61434/terrain?lat=35.4&lon=138.73&size=1100&levelMode=manual&z=5&fullscreen=1',{timeout:120000});
async function ready(z,mode,line){await page.waitForFunction(({z,mode,line})=>data.coverage.zoom===z&&data.coverage.levelMode===mode&&(!line||data.overlays.lineLevel===line)&&!lodBusy,{z,mode,line},{timeout:120000});}
await ready(5,'manual');console.log('手動 5 を確認');assert.deepEqual(await page.evaluate(()=>Object.keys(data.coverage.levels)),['5']);
await page.evaluate(()=>{const el=document.getElementById('zoom');el.value=baseSpan/1000*Math.max(1,canvas.clientWidth/canvas.clientHeight)*1.5/1100;el.dispatchEvent(new Event('input'));});
await ready(5,'manual',7);
await page.selectOption('#terrainLevel','7');await ready(7,'manual',7);console.log('手動 7 を確認');assert.deepEqual(await page.evaluate(()=>Object.keys(data.coverage.levels)),['7']);
await page.selectOption('#terrainLevel','auto');await ready(7,'auto',7);console.log('自動 7 を確認');
await page.evaluate(()=>{const el=document.getElementById('zoom');el.value=baseSpan/1000*Math.max(1,canvas.clientWidth/canvas.clientHeight)*1.5/4000;el.dispatchEvent(new Event('input'));});
await ready(5,'auto',5);console.log('自動 5 を確認');
assert.deepEqual(await page.evaluate(()=>data.overlays.lineLevels),[4,5,6,7,8,9,10,11,12,13,14]);
assert.ok(await page.evaluate(()=>data.overlays.counts.river>0&&data.overlays.counts.trail>0&&data.overlays.counts.boundary>0));
await page.selectOption('#terrainLevel','5');await ready(5,'manual',5);
await page.reload({timeout:120000});await ready(5,'manual');console.log('手動 5 を確認');assert.equal(await page.inputValue('#terrainLevel'),'5');
assert.equal(await page.evaluate(()=>gl.getError()),0);assert.deepEqual(errors,[]);
console.log('レベル 5・7 の手動固定、自動切り替え、線データ切り替え、設定復元を確認しました。');
}finally{await browser.close();}})().catch(e=>{console.error(e);process.exitCode=1;});
