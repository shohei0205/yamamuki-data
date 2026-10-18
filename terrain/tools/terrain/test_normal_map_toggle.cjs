// 法線マップを同じ地形・視点で切り替え、追加取得せず比較できることを確認する。
const assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{
 const browser=await chromium.launch({headless:true,channel:'msedge'});
 try{
  const page=await browser.newPage({viewport:{width:1100,height:800}}),errors=[],normalRequests=[];
  page.on('pageerror',e=>errors.push(e.message));page.on('request',r=>{if(r.url().includes('/sqlite/normals'))normalRequests.push(r.url());});
  const url='http://127.0.0.1:61434/terrain?lat=36.32155&lon=137.6476&size=30&fullscreen=1&levelMode=manual&z=4';
  await page.goto(url);
  await page.waitForFunction(()=>typeof normalMapCount!=='undefined'&&normalMapCount===4&&!data.coverage.overviewFallback&&!lodBusy&&data.overlays?.nationalLines,null,{timeout:120000});
  const snap=()=>page.evaluate(()=>{draw();const pixels=new Uint8Array(canvas.width*canvas.height*4);gl.readPixels(0,0,canvas.width,canvas.height,gl.RGBA,gl.UNSIGNED_BYTE,pixels);let hash=2166136261;for(const value of pixels)hash=Math.imul(hash^value,16777619);return {hash:hash>>>0,indices:data.indices.length,vertices:data.mesh.length,normal:gl.getUniform(program,uniforms.useNormalMap),error:gl.getError()};});
  assert.equal(await page.locator('#normalMap').isEnabled(),true);assert.equal(await page.locator('#normalMap').isChecked(),true);
  const on=await snap();assert.equal(on.normal,true);
  await page.screenshot({path:'build/terrain-national/normal-map-on.png'});
  const requests=normalRequests.length;
  await page.uncheck('#normalMap');const off=await snap();assert.equal(off.normal,false);assert.notEqual(off.hash,on.hash);
  assert.equal(off.indices,on.indices);assert.equal(off.vertices,on.vertices);assert.equal(normalRequests.length,requests);
  await page.screenshot({path:'build/terrain-national/normal-map-off.png'});
  await page.check('#normalMap');const again=await snap();assert.equal(again.hash,on.hash);assert.equal(again.error,0);
  await page.uncheck('#normalMap');await page.goto(url);
  await page.waitForFunction(()=>typeof normalMapCount!=='undefined'&&normalMapCount===4&&document.getElementById('stats').textContent.includes('三角形'),null,{timeout:120000});
  assert.equal(await page.locator('#normalMap').isChecked(),false);
  await page.goto('http://127.0.0.1:61434/qem-preview?z=11&fullscreen=1');
  await page.waitForFunction(()=>document.getElementById('stats').textContent.includes('三角形'));
  assert.equal(await page.locator('#normalMap').isEnabled(),true);
  assert.equal(await page.locator('#normalMap').isChecked(),false);
  assert.match(await page.locator('#normalMapLabel').textContent(),/未収録/);
  await page.check('#normalMap');await page.uncheck('#normalMap');
  assert.equal(await page.locator('#normalMap').isChecked(),false);
  assert.deepEqual(errors,[]);console.log(JSON.stringify({on,off,sameGeometry:true,noExtraNormalDownload:true,preferenceRetained:true,missingMapUncheckedAndOperable:true,errors}));
 }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
