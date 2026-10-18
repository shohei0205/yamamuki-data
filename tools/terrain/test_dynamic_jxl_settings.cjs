// 動的圧縮と元 PNG への切替で、陰影の選択・光源・強さを保持する。
const assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{
 const browser=await chromium.launch({headless:true,channel:'msedge'});
 try{
  const page=await browser.newPage(),errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.goto('http://127.0.0.1:61440/heightmap?level=10');
  await page.waitForFunction(()=>window.heightmapStatus?.level===10);
  await page.selectOption('#normalResolution','256');
  await page.waitForFunction(()=>window.heightmapStatus?.normalSize===256);
  const settings=()=>page.evaluate(()=>Object.fromEntries(['normal','normalStep','light','strength'].map(id=>[id,document.getElementById(id).value])));
  for(const mode of ['face','gpu','png']){
   await page.selectOption('#normal',mode);await page.selectOption('#normalStep','4');
   await page.locator('#light').fill('120');await page.locator('#strength').fill('0.37');
   const before=await settings(),previous=await page.evaluate(()=>dynamicJXL?.id);
   await page.click('#jxlApply');
   await page.waitForFunction(id=>dynamicJXL?.id&&dynamicJXL.id!==id&&!document.getElementById('jxlApply').disabled,previous);
   assert.deepEqual(await settings(),before);
   for(const original of [true,false]){
    await page.setChecked('#jxlOriginal',original);
    await page.waitForFunction(codec=>window.heightmapStatus?.heightCodec===codec,original?'png':'dynamic-jxl');
    assert.deepEqual(await settings(),before);
   }
   assert.equal(await page.evaluate(()=>window.heightmapStatus.normal),mode);
   assert.equal(await page.evaluate(()=>window.heightmapStatus.glError),0);
  }
  assert.deepEqual(errors,[]);console.log('圧縮・元 PNG 切替で全陰影モードと光源・強さを保持: OK');
 }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exit(1);});
