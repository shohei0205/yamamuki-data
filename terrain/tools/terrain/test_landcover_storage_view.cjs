const {chromium}=require('playwright'),assert=require('node:assert/strict');
(async()=>{const browser=await chromium.launch({channel:'msedge',headless:true});try{
 const page=await browser.newPage({viewport:{width:1440,height:1000}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.goto('http://127.0.0.1:61452/landcover_compare.html');await page.waitForFunction(()=>data?.complete);
 assert.equal(await page.evaluate(()=>data.rows.length),90);
 for(const region of ['tsukuba','fuji','yari'])for(const size of ['512','1024','2048']){
  await page.selectOption('#region',region);await page.selectOption('#size',size);
  for(const mode of ['rgb','palette64','palette128','palette256','class','two','jxl0.3','jxl1.0','astc6','astc8']){
   await page.selectOption('#mode',mode);await page.waitForFunction(()=>[...document.images].every(i=>i.complete&&i.naturalWidth===+$('size').value));
  }
 }
 await page.selectOption('#region','tsukuba');await page.selectOption('#size','1024');await page.selectOption('#mode','palette64');
 await page.waitForFunction(()=>[...document.images].every(i=>i.complete&&i.naturalWidth===1024));
 await page.screenshot({path:'build/landcover-compare/view.png',fullPage:true});assert.deepEqual(errors,[]);console.log('全90比較画像・地域と解像度の切替・画面エラーなしを確認');
}finally{await browser.close()}})().catch(e=>{console.error(e);process.exit(1)});
