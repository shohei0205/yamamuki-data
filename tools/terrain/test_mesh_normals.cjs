// LOD ごとの法線の選択・切り替え・取得量と、同じ視点での描画差を確かめる。
const assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{const browser=await chromium.launch({headless:true,channel:'msedge'});
try{
 const page=await browser.newPage({viewport:{width:1200,height:800}}),errors=[];
 page.on('pageerror',e=>errors.push(e.message));
 await page.goto('http://127.0.0.1:61440/heightmap?geometry=polygon9&meshCodec=meshopt-f32-zstd&normalCodec=astc4-zstd');
 await page.waitForFunction(()=>window.heightmapStatus?.geometry==='polygon');
 const report=await page.evaluate(async()=>await(await fetch('/mesh-normals/report.json')).json());
 assert.equal(report.state,'complete');assert.equal(report.levels.length,11);
 for(const row of report.levels){assert.equal(row.normalVariants.length,10);assert.equal(row.compressed.length,30);assert.ok(row.coveredPixels>0);}
 const camera=()=>page.evaluate(()=>({target:[...target],distance,bearing,tilt,light:document.getElementById('light').value}));
 const before=await camera();
 const pixels=()=>page.evaluate(()=>{draw();const a=new Uint8Array(canvas.width*canvas.height*4);gl.readPixels(0,0,canvas.width,canvas.height,gl.RGBA,gl.UNSIGNED_BYTE,a);let sum=0;for(let i=0;i<a.length;i+=4)sum+=a[i];return sum;});
 const shared=await pixels();await page.screenshot({path:'build/terrain-codec-comparison/mesh-normal-shared-L9.png'});
 await page.selectOption('#normalSource','mesh');
 await page.waitForFunction(()=>heightmapStatus.normalSource==='mesh'&&!document.getElementById('normalSource').disabled);
 assert.notEqual(await pixels(),shared);assert.deepEqual(await camera(),before);
 await page.screenshot({path:'build/terrain-codec-comparison/mesh-normal-matched-L9.png'});
 const results=[];
 for(const level of [4,5,6,7,8,9,10,11,12,13,14]){
  await page.selectOption('#geometryMode','polygon'+level);
  await page.waitForFunction(level=>heightmapStatus.level===level&&heightmapStatus.normalSource==='mesh'&&heightmapStatus.normalLevel===level&&!document.getElementById('recommended').disabled,level,{timeout:90000});
  const state=await page.evaluate(()=>heightmapStatus),row=report.levels.find(r=>r.level===level).compressed.find(r=>r.size===1024&&r.block===4);
  assert.equal(state.normalBytes,row.ktx2ZstdBytes);assert.equal(state.glError,0);assert.deepEqual(await camera(),before);results.push({level,normalBytes:state.normalBytes});
 }
 // PNG と ASTC の両経路が LOD ごとの画像を選び、再選択で取得し直さない。
 await page.selectOption('#normalCodec','png');await page.waitForFunction(()=>heightmapStatus.normalCodec==='png'&&!document.getElementById('normalCodec').disabled);
 const png=report.levels.find(r=>r.level===14).normalVariants.find(r=>r.size===1024);assert.equal((await page.evaluate(()=>heightmapStatus)).normalBytes,png.bytes);
 await page.selectOption('#normalCodec','astc4-zstd');await page.waitForFunction(()=>heightmapStatus.normalCodec==='astc4-zstd'&&!document.getElementById('normalCodec').disabled);
 const bytes=await page.evaluate(()=>heightmapStatus.bytes);
 await page.selectOption('#normalSource','shared');await page.waitForFunction(()=>heightmapStatus.normalSource==='shared'&&!document.getElementById('normalSource').disabled);
 await page.selectOption('#normalSource','mesh');await page.waitForFunction(()=>heightmapStatus.normalSource==='mesh'&&!document.getElementById('normalSource').disabled);
 assert.equal(await page.evaluate(()=>heightmapStatus.bytes),bytes);assert.deepEqual(errors,[]);
 console.log(JSON.stringify({results,sameCamera:true,sharedAndMatchedDiffer:true,noRepeatedDownloads:true}));
}finally{await browser.close();}})().catch(e=>{console.error(e);process.exitCode=1;});
