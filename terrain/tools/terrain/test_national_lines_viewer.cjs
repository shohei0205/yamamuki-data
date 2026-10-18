// 全国の複数地域と広域表示で 3 レイヤー・11 段階・地形との独立性を確認する。
const assert=require('node:assert/strict');
const {chromium}=require('playwright');
(async()=>{const browser=await chromium.launch({headless:true,channel:'msedge'});
try{
 const page=await browser.newPage({viewport:{width:1100,height:800}});const errors=[],results=[];
 page.on('pageerror',e=>errors.push(e.message));
 await page.addInitScript(()=>{localStorage.setItem('terrain-preview-occludeLayers','false');localStorage.setItem('terrain-preview-level','auto');});
 async function region(name,lat,lon,size,screenshot){
  console.log('表示確認',name);
  await page.goto(`http://127.0.0.1:61434/terrain?lat=${lat}&lon=${lon}&size=${size}&levelMode=auto&fullscreen=1`,{timeout:120000});
  await page.waitForFunction(()=>data.overlays?.nationalLines&&!lodBusy,null,{timeout:60000});
  const result=await page.evaluate(()=>({counts:data.overlays.counts,levels:data.overlays.lineLevels,lineLevel:data.overlays.lineLevel,terrainLevel:data.coverage.zoom,gl:gl.getError()}));
  assert.deepEqual(result.levels,[4,5,6,7,8,9,10,11,12,13,14]);assert.equal(result.gl,0);assert.ok(result.counts.river>0);
  if(screenshot)await page.screenshot({path:`build/terrain-national/${screenshot}.png`});results.push({name,...result});return result;
 }
 const yari=await region('槍ヶ岳',36.32155,137.6476,30,'yari-national-lines');assert.ok(yari.counts.trail>0&&yari.counts.boundary>0);
 const kyushu=await region('阿蘇',32.88,131.1,60);assert.ok(kyushu.counts.trail>0);
 const hokkaido=await region('大雪山',43.66,142.85,40,'hokkaido-national-lines');assert.equal(hokkaido.counts.boundary,0);
 await region('全国',37,138,3600,'japan-national-lines');
 await page.goto('http://127.0.0.1:61434/terrain?lat=35.4&lon=138.73&size=220&levelMode=manual&z=8&fullscreen=1',{timeout:120000});
 await page.waitForFunction(()=>data.overlays?.nationalLines&&!lodBusy,null,{timeout:60000});
 await page.evaluate(()=>{document.getElementById('terrainLayer').checked=false;document.getElementById('occludeLayers').checked=false;draw();});
 async function size(size,level){
  await page.evaluate(size=>{const el=document.getElementById('zoom');el.value=baseSpan/1000*Math.max(1,canvas.clientWidth/canvas.clientHeight)*1.5/size;el.dispatchEvent(new Event('input'));},size);
  await page.waitForFunction(level=>data.overlays.lineLevel===level&&!lodBusy,level,{timeout:60000});
  assert.equal(await page.evaluate(()=>data.coverage.zoom),8);assert.equal(await page.isChecked('#terrainLayer'),false);
 }
 await size(60,11);await size(120,10);await size(400,9);assert.deepEqual(errors,[]);
 console.log(JSON.stringify({results,manualTerrainLevel:8,lineLevelsVisited:[11,10,9],errors},null,2));
}finally{await browser.close();}})().catch(e=>{console.error(e);process.exitCode=1;});
