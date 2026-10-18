// レベル変更と再読み込みをまたいで滑らかな陰影の選択を保持する。
const assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{
 const browser=await chromium.launch({headless:true,channel:'msedge'});
 try{
  const page=await browser.newPage(),errors=[];
  page.on('pageerror',e=>errors.push(e.message));
  await page.goto('http://127.0.0.1:61434/qem-preview?z=11&fullscreen=1');
  await page.waitForFunction(()=>document.getElementById('stats').textContent.includes('三角形'));
  await page.uncheck('#smooth');
  await page.selectOption('#terrainLevel','9');
  await page.waitForFunction(()=>data.coverage.zoom===9&&document.getElementById('stats').textContent.includes('三角形'));
  assert.equal(await page.locator('#smooth').isChecked(),false);
  assert.equal(await page.evaluate(()=>gl.getUniform(program,uniforms.smoothShading)),false);
  await page.check('#smooth');
  await page.selectOption('#terrainLevel','11');
  await page.waitForFunction(()=>data.coverage.zoom===11&&document.getElementById('stats').textContent.includes('三角形'));
  assert.equal(await page.locator('#smooth').isChecked(),true);
  await page.uncheck('#smooth');await page.reload();
  await page.waitForFunction(()=>document.getElementById('stats').textContent.includes('三角形'));
  assert.equal(await page.locator('#smooth').isChecked(),false);
  assert.deepEqual(errors,[]);console.log('滑らかな陰影: レベル変更・再読み込み後のオン／オフ保持を確認');
 }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
