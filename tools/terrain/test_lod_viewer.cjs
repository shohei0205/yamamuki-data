// 実データで自動縮尺、応答の陳腐化、読み込み失敗時の表示維持を検証する。
const assert=require('node:assert/strict');
const {chromium}=require('playwright');
async function main(){
  const browser=await chromium.launch({headless:true,channel:'msedge'});
  try{
    const page=await browser.newPage({viewport:{width:1024,height:1024}});
    const errors=[];page.on('pageerror',e=>errors.push(e.message));
    let hold=false,failNext=false,release;
    const gate=new Promise(resolve=>{release=resolve;});
    await page.route('**/terrain-data?**',async route=>{
      if(hold&&new URL(route.request().url()).searchParams.get('z')==='10'){hold=false;await gate;}
      if(failNext){failNext=false;return route.abort();}
      return route.continue();
    });
    await page.goto('http://127.0.0.1:61434/terrain?lat=35.40&lon=138.73&size=3600');
    await page.waitForFunction(()=>document.querySelector('#stats').textContent.includes('三角形'));
    await page.evaluate(()=>{
      window.originalCanvas=canvas;window.appliedLevels=[];
      new MutationObserver(()=>{if(document.querySelector('#lodStatus').textContent.includes('表示範囲に合わせて'))window.appliedLevels.push(data.coverage.zoom);}).observe(document.querySelector('#lodStatus'),{childList:true});
      document.querySelector('#bearing').value='27';document.querySelector('#tilt').value='42';draw();
    });
    async function zoom(value){await page.evaluate(value=>{document.querySelector('#zoom').value=value;document.querySelector('#zoom').dispatchEvent(new Event('input'));},value);}
    async function level(value){await page.waitForFunction(value=>data.coverage.zoom===value&&!lodBusy,value,{timeout:90000});}
    const center=await page.evaluate(()=>viewCenter.slice(0,2));
    assert.equal(await page.evaluate(()=>data.coverage.zoom),4);
    await zoom(20);await level(8);
    hold=true;const pending=page.waitForRequest(r=>r.url().includes('/terrain-data?')&&new URL(r.url()).searchParams.get('z')==='10');
    await zoom(200);await pending;
    assert.equal(await page.evaluate(()=>data.coverage.zoom),8);
    assert.equal(await page.evaluate(()=>canvas===window.originalCanvas),true);
    await zoom(1000);release();await level(14);
    assert.equal(await page.evaluate(()=>window.appliedLevels.includes(10)),false);
    assert.deepEqual(await page.evaluate(()=>viewCenter.slice(0,2)),center);
    assert.equal(await page.locator('#bearing').inputValue(),'27');assert.equal(await page.locator('#tilt').inputValue(),'42');
    const oldLon=await page.evaluate(()=>data.coverage.longitude);
    await page.mouse.move(350,400);await page.mouse.down();await page.mouse.move(3350,400,{steps:12});await page.mouse.up();
    await page.waitForFunction(lon=>Math.abs(data.coverage.longitude-lon)>.05&&!lodBusy,oldLon,{timeout:90000});
    assert.equal(await page.evaluate(()=>data.coverage.zoom),14);
    failNext=true;await zoom(80);
    await page.waitForFunction(()=>document.querySelector('#lodStatus').textContent.includes('現在の表示を残しています'),null,{timeout:90000});
    assert.equal(await page.evaluate(()=>data.coverage.zoom),14);
    await zoom(1);await level(4);
    assert.equal(await page.evaluate(()=>canvas===window.originalCanvas),true);
    assert.ok(await page.evaluate(()=>lodCache.size<=2));assert.equal(await page.evaluate(()=>gl.getError()),0);
    assert.deepEqual(errors,[]);
    console.log('全国→細部→全国、移動先の取得、古い応答の破棄、失敗時の表示維持、方角と傾き、同じ描画面を確認しました。');
  }finally{await browser.close();}
}
main().catch(e=>{console.error(e);process.exitCode=1;});
