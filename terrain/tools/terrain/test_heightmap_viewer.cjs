// PNG の法線・GPU 法線・面の陰影を、同じ形状と視点で比較できることを確認する。
const assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{const browser=await chromium.launch({headless:true,channel:'msedge'});
try{
 const page=await browser.newPage({viewport:{width:1200,height:800}}),errors=[],requests=[];
 page.on('pageerror',e=>errors.push(e.message));page.on('request',r=>requests.push(r.url()));
 await page.goto('http://127.0.0.1:61440/heightmap');
 await page.waitForFunction(()=>window.heightmapStatus?.triangles>0,null,{timeout:60000});
 await page.waitForTimeout(1200);
 const initial=await page.evaluate(()=>({...heightmapStatus,loadMilliseconds:heightmapLoadMilliseconds}));
 assert.equal(initial.glError,0);assert.equal(initial.normal,'png');
 assert.equal(await page.evaluate(()=>heightmapMetadata.pngMaxHeightErrorMeters),0);
 await page.screenshot({path:'build/terrain-heightmap-yari/normal-png.png'});
 const count=requests.length;
 await page.selectOption('#normal','gpu');await page.waitForTimeout(500);
 const gpu=await page.evaluate(()=>heightmapStatus);
 assert.equal(gpu.normal,'gpu');assert.equal(gpu.triangles,initial.triangles);assert.equal(gpu.glError,0);
 await page.screenshot({path:'build/terrain-heightmap-yari/normal-gpu.png'});
 await page.selectOption('#normal','face');await page.waitForTimeout(200);
 assert.equal(requests.length,count);
 await page.selectOption('#step','16');await page.waitForFunction(()=>heightmapStatus.step===16);
 assert.ok((await page.evaluate(()=>heightmapStatus.triangles))<initial.triangles);
 await page.selectOption('#normal','png');await page.selectOption('#normalResolution','2048');
 await page.waitForFunction(()=>!document.getElementById('normalResolution').disabled);
 assert.equal(requests.filter(u=>u.endsWith('/normals/2048.png')).length,1);
 await page.selectOption('#normalResolution','1024');await page.waitForTimeout(200);
 assert.equal(requests.filter(u=>u.endsWith('/normals/1024.png')).length,1);
 const fixedTriangles=await page.evaluate(()=>heightmapStatus.triangles);
 for(const size of [2048,1024,512,256,128,64,32,16,8,4]){
  await page.selectOption('#normalResolution',String(size));
  await page.waitForFunction(size=>!document.getElementById('normalResolution').disabled&&heightmapStatus.normalSize===size,size);
  assert.equal(await page.evaluate(()=>heightmapStatus.glError),0);
  assert.equal(await page.evaluate(()=>heightmapStatus.triangles),fixedTriangles);
 }
 const original=await page.evaluate(()=>({target:[...target],distance,bearing,tilt}));
 await page.mouse.move(500,400);await page.mouse.down();await page.mouse.move(560,420);await page.mouse.up();
 assert.notDeepEqual(await page.evaluate(()=>target),original.target);
 await page.mouse.wheel(0,-200);await page.waitForTimeout(100);
 assert.ok(await page.evaluate(()=>distance)<original.distance);
 await page.mouse.down({button:'right'});await page.mouse.move(620,450);await page.mouse.up({button:'right'});
 assert.notEqual(await page.evaluate(()=>bearing),original.bearing);
 assert.notEqual(await page.evaluate(()=>tilt),original.tilt);
 await page.click('#reset');assert.equal(await page.evaluate(()=>distance),original.distance);
 const levels=[];
 await page.selectOption('#step','auto');
 for(let level=13;level>=4;level--){
  await page.selectOption('#dataLevel',String(level));
  await page.waitForFunction(level=>heightmapStatus.level===level&&!document.getElementById('dataLevel').disabled,level);
  const state=await page.evaluate(()=>({...heightmapStatus,target:[...target],distance,bearing,tilt}));
  assert.equal(state.glError,0);assert.ok(state.triangles>0);assert.deepEqual(state.target,original.target);assert.equal(state.distance,original.distance);
  assert.equal(state.normalSize,4);
  levels.push({level,bytes:state.selectedBytes,triangles:state.triangles});
 }
 await page.selectOption('#dataLevel','14');await page.waitForFunction(()=>heightmapStatus.level===14&&!document.getElementById('dataLevel').disabled);
 assert.equal(requests.filter(u=>u.endsWith('/normals/4.png')).length,1);
 assert.deepEqual(errors,[]);console.log(JSON.stringify({initial,gpu,levels,allChecksPassed:true}));
}finally{await browser.close();}})().catch(e=>{console.error(e);process.exitCode=1;});
