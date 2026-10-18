// 焼き込み法線が詳細を保持し、各 LOD で正しい基準から復元されることを確認する。
const assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{const browser=await chromium.launch({headless:true,channel:'msedge'});
try{
 const page=await browser.newPage({viewport:{width:1000,height:700}}),errors=[];
 page.on('pageerror',e=>errors.push(e.message));
 await page.goto('http://127.0.0.1:61440/heightmap?geometry=polygon9&meshCodec=meshopt-f32-zstd&normalCodec=png&normalSource=shared');
 await page.waitForFunction(()=>window.heightmapStatus?.geometry==='polygon');
 let report;const deadline=Date.now()+300000;
 do{report=await page.evaluate(async()=>await(await fetch('/baked-normals/report.json')).json());if(report.state==='complete')break;await page.waitForTimeout(1000);}while(Date.now()<deadline);
 assert.equal(report.state,'complete');assert.equal(report.levels.length,11);
 const camera=()=>page.evaluate(()=>({target:[...target],distance,bearing,tilt,light:document.getElementById('light').value}));
 const before=await camera();
 await page.evaluate(()=>{window.bakeReference=()=>{draw();const p=new Uint8Array(canvas.width*canvas.height*4);gl.readPixels(0,0,canvas.width,canvas.height,gl.RGBA,gl.UNSIGNED_BYTE,p);return p;};window.sharedPixels=bakeReference();});
 await page.screenshot({path:'build/terrain-codec-comparison/baked-compare-shared-L9.png'});
 const difference=()=>page.evaluate(()=>{const p=bakeReference();let sum=0;for(let i=0;i<p.length;i++)if(i%4!==3)sum+=Math.abs(p[i]-sharedPixels[i]);return sum/(p.length*0.75);});
 await page.selectOption('#normalSource','baked');await page.waitForFunction(()=>heightmapStatus.normalSource==='baked'&&!document.getElementById('normalSource').disabled);
 const bakeError=await difference();assert.equal((await page.evaluate(()=>heightmapStatus)).glError,0);assert.deepEqual(await camera(),before);
 await page.screenshot({path:'build/terrain-codec-comparison/baked-compare-baked-L9.png'});
 await page.selectOption('#normalSource','mesh');await page.waitForFunction(()=>heightmapStatus.normalSource==='mesh'&&!document.getElementById('normalSource').disabled);
 const shapeOnlyError=await difference();assert.ok(bakeError<shapeOnlyError,{bakeError,shapeOnlyError});assert.ok(bakeError<2.5,{bakeError});
 await page.screenshot({path:'build/terrain-codec-comparison/baked-compare-shape-only-L9.png'});
 await page.selectOption('#normalSource','baked');await page.waitForFunction(()=>heightmapStatus.normalSource==='baked'&&!document.getElementById('normalSource').disabled);
 await page.selectOption('#normalCodec','astc4-zstd');await page.waitForFunction(()=>heightmapStatus.normalCodec==='astc4-zstd'&&!document.getElementById('normalCodec').disabled);
 const results=[];
 for(const level of [4,5,6,7,8,9,10,11,12,13,14]){
  await page.selectOption('#geometryMode','polygon'+level);await page.waitForFunction(level=>heightmapStatus.level===level&&heightmapStatus.normalLevel===level&&heightmapStatus.normalSource==='baked'&&!document.getElementById('recommended').disabled,level,{timeout:90000});
  const row=report.levels.find(r=>r.level===level),state=await page.evaluate(()=>heightmapStatus);
  assert.equal(state.glError,0);assert.equal(state.normalBytes,row.compressed.find(r=>r.size===1024&&r.block===4).ktx2ZstdBytes);assert.deepEqual(await camera(),before);results.push({level,bytes:state.normalBytes});
 }
 for(const codec of ['astc4','astc6','astc6-zstd','astc8','astc8-zstd','png']){
  await page.selectOption('#normalCodec',codec);await page.waitForFunction(codec=>heightmapStatus.normalCodec===codec&&!document.getElementById('normalCodec').disabled,codec);
  assert.equal((await page.evaluate(()=>heightmapStatus)).glError,0);
 }
 assert.deepEqual(errors,[]);console.log(JSON.stringify({results,bakeError,shapeOnlyError,detailPreserved:true,sameCamera:true}));
}finally{await browser.close();}})().catch(e=>{console.error(e);process.exitCode=1;});
