// 同じ視点・法線 PNG・光源でポリゴンとハイトマップを切り替える。
const assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{const browser=await chromium.launch({headless:true,channel:'msedge'});
try{
 const page=await browser.newPage({viewport:{width:1200,height:800}}),errors=[],requests=[];
 page.on('pageerror',e=>errors.push(e.message));page.on('request',r=>requests.push(r.url()));
 await page.goto('http://127.0.0.1:61440/heightmap?geometry=polygon9&level=9');
 await page.waitForFunction(()=>window.heightmapStatus?.geometry==='polygon'&&heightmapStatus.level===9,null,{timeout:60000});
 assert.equal(requests.some(u=>u.includes('/levels/9/height.png')),false);
 const settings=()=>page.evaluate(()=>({target:[...target],distance,bearing,tilt,light:document.getElementById('light').value,normal:document.getElementById('normalResolution').value}));
 const initial=await settings();const results=[];
 const pixelHash=async()=>require('node:crypto').createHash('sha256').update(await page.screenshot({clip:{x:0,y:160,width:1200,height:550}})).digest('hex');
 for(const level of [9,10]){
  if(level===10){await page.selectOption('#dataLevel','10');await page.waitForFunction(()=>heightmapStatus.level===10&&!document.getElementById('geometryMode').disabled);}
  await page.selectOption('#geometryMode','polygon'+level);
  await page.waitForFunction(level=>heightmapStatus.geometry==='polygon'&&heightmapStatus.level===level&&!document.getElementById('geometryMode').disabled,level);
  const poly=await page.evaluate(()=>heightmapStatus);assert.equal(poly.glError,0);
  assert.equal(poly.triangles,level===9?59456:118878);
  assert.equal(poly.geometryBytes,level===9?393387:804971);
  assert.equal(poly.auxiliaryBytes,0);assert.deepEqual(await settings(),initial);
  await page.screenshot({path:`build/terrain-heightmap-yari/polygon-${level}-normal.png`});
  const normalHash=await pixelHash();
  await page.selectOption('#normal','face');await page.waitForTimeout(100);
  await page.screenshot({path:`build/terrain-heightmap-yari/polygon-${level}-face.png`});
  assert.notEqual(await pixelHash(),normalHash);
  await page.selectOption('#normal','png');
  await page.selectOption('#geometryMode','height');
  await page.waitForFunction(()=>heightmapStatus.geometry==='height'&&!document.getElementById('geometryMode').disabled);
  const height=await page.evaluate(()=>heightmapStatus);assert.equal(height.level,level);assert.equal(height.glError,0);
  assert.equal(height.normalBytes,poly.normalBytes);assert.deepEqual(await settings(),initial);
  await page.screenshot({path:`build/terrain-heightmap-yari/height-${level}-normal.png`});
  results.push({level,polygon:poly,heightmap:height});
 }
 await page.selectOption('#dataLevel','12');await page.waitForFunction(()=>heightmapStatus.level===12&&!document.getElementById('geometryMode').disabled);
 await page.selectOption('#geometryMode','polygon9');await page.waitForFunction(()=>heightmapStatus.geometry==='polygon'&&heightmapStatus.level===9);
 await page.selectOption('#geometryMode','height');await page.waitForFunction(()=>heightmapStatus.geometry==='height'&&heightmapStatus.level===12);
 assert.equal(requests.filter(u=>u.endsWith('/polygon/9')).length,1);
 assert.equal(requests.filter(u=>u.endsWith('/normals/1024.png')).length,1);
 assert.deepEqual(errors,[]);console.log(JSON.stringify({results,sameCameraLightAndNormal:true,noRepeatedDownloads:true}));
}finally{await browser.close();}})().catch(e=>{console.error(e);process.exitCode=1;});
