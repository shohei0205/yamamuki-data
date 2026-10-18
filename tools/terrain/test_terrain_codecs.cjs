// Rust が生成した実ファイルをブラウザーで復元し、誤差・同じ視点・GPU 表示を確認する。
const assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{const browser=await chromium.launch({headless:true,channel:'msedge'});
try{
 const page=await browser.newPage({viewport:{width:1200,height:800}}),errors=[];
 page.on('pageerror',e=>errors.push(e.message));page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
 await page.goto('http://127.0.0.1:61440/heightmap?geometry=polygon9&level=9&meshCodec=meshopt-zstd&normalCodec=astc4-zstd');
 await page.waitForFunction(()=>window.heightmapStatus?.geometry==='polygon'&&heightmapStatus.meshCodec==='meshopt-zstd',null,{timeout:90000});
 const original=await page.evaluate(()=>({target:[...target],distance,bearing,tilt,light:document.getElementById('light').value}));
 const snapshot=()=>page.evaluate(()=>({...heightmapStatus,target:[...target],distance,bearing,tilt,light:document.getElementById('light').value}));
 const meshResults=[];
 for(const level of [9,10]){
  await page.selectOption('#geometryMode','polygon'+level);
  await page.waitForFunction(level=>heightmapStatus.level===level&&!document.getElementById('geometryMode').disabled,level);
  for(const codec of ['meshopt-f32','meshopt-f32-zstd','meshopt','meshopt-zstd','hierarchy','original']){
   await page.selectOption('#meshCodec',codec);await page.waitForFunction(codec=>heightmapStatus.meshCodec===codec&&!document.getElementById('geometryMode').disabled,codec);
   const state=await snapshot();assert.equal(state.glError,0);assert.equal(state.triangles,level===9?59456:118878);
   assert.deepEqual({target:state.target,distance:state.distance,bearing:state.bearing,tilt:state.tilt,light:state.light},original);
   meshResults.push({level,codec,bytes:state.geometryBytes,triangles:state.triangles});
  }
 }
 const normalResults=[];
 for(const codec of ['astc4','astc4-zstd','astc6','astc6-zstd','astc8','astc8-zstd','png']){
  await page.selectOption('#normalCodec',codec);await page.waitForFunction(codec=>heightmapStatus.normalCodec===codec&&!document.getElementById('normalCodec').disabled,codec);
  const state=await snapshot();assert.equal(state.glError,0);assert.equal(state.triangles,118878);normalResults.push({codec,bytes:state.normalBytes,hardware:state.normalHardware});
 }
 await page.selectOption('#meshCodec','meshopt-f32-zstd');await page.waitForFunction(()=>!document.getElementById('geometryMode').disabled);
 await page.selectOption('#normalCodec','astc4-zstd');await page.waitForFunction(()=>!document.getElementById('normalCodec').disabled);
 await page.screenshot({path:'build/terrain-codec-comparison/meshopt-astc4.png'});
 await page.selectOption('#geometryMode','height');await page.waitForFunction(()=>heightmapStatus.geometry==='height'&&!document.getElementById('geometryMode').disabled);
 await page.selectOption('#dataLevel','10');await page.waitForFunction(()=>heightmapStatus.level===10&&!document.getElementById('geometryMode').disabled);
 const heightResults=[];
 for(const codec of ['ymd-xz-1','ymd-xz-5','ymd-gzip-1','lerc-0','lerc-1','lerc-2p5','lerc-5','delta-zstd','residual-zstd']){
  await page.selectOption('#heightCodec',codec);await page.waitForFunction(codec=>heightmapStatus.heightCodec===codec&&!document.getElementById('geometryMode').disabled,codec,{timeout:90000});
  const state=await snapshot();assert.equal(state.glError,0);assert.equal(state.level,10);heightResults.push({codec,bytes:state.geometryBytes});
 }
 const precision=await page.evaluate(async()=>{
  const response=await fetch('/levels/10/height.png'),image=await createImageBitmap(await response.blob(),{colorSpaceConversion:'none'}),c=new OffscreenCanvas(image.width,image.height),ctx=c.getContext('2d');ctx.drawImage(image,0,0);const pixels=ctx.getImageData(0,0,image.width,image.height).data;image.close();
  const result=[];for(const codec of ['ymd-xz-1','ymd-xz-5','ymd-gzip-1','lerc-0','lerc-1','lerc-2p5','lerc-5','delta-zstd','residual-zstd']){
   const d=await TerrainCodecs.loadHeight(10,codec);let max=0,missing=0;
   for(let i=0;i<d.values.length;i++){const valid=pixels[i*4+2]>0;if(valid!==Number.isFinite(d.values[i])){missing++;continue;}if(valid)max=Math.max(max,Math.abs(d.values[i]-(pixels[i*4]+256*pixels[i*4+1]+heightmapMetadata.heightOffset)));}
   result.push({codec,max,missing,expected:d.row.maxErrorMeters});
  }return result;
 });
 for(const p of precision){assert.equal(p.missing,0);assert.ok(Math.abs(p.max-p.expected)<0.001,JSON.stringify(p));}
 await page.screenshot({path:'build/terrain-codec-comparison/height-residual-astc4.png'});
 // 別の設定から一括適用しても、視点と光源、ハイトマップの比較レベルは保持する。
 await page.selectOption('#normal','face');
 const beforeRecommended=await snapshot();
 await page.click('#recommended');
 await page.waitForFunction(()=>heightmapStatus.geometry==='polygon'&&heightmapStatus.meshCodec==='meshopt-f32-zstd'&&!document.getElementById('recommended').disabled);
 const recommended=await snapshot();assert.equal(recommended.level,10);assert.equal(recommended.normalSource,'baked');assert.equal(recommended.normal,'png');assert.equal(recommended.normalSize,1024);assert.equal(recommended.normalCodec,'astc4-zstd');assert.equal(recommended.heightCodec,'ymd-xz-1');assert.equal(recommended.glError,0);
 for(const key of ['target','distance','bearing','tilt','light'])assert.deepEqual(recommended[key],beforeRecommended[key]);
 await page.screenshot({path:'build/terrain-codec-comparison/recommended.png'});
 await page.selectOption('#geometryMode','height');await page.waitForFunction(()=>heightmapStatus.geometry==='height'&&!document.getElementById('recommended').disabled);assert.equal((await snapshot()).level,10);
 await page.goto('http://127.0.0.1:61440/codec-report');await page.waitForFunction(()=>document.getElementById('status').textContent.includes('計測完了'));
 assert.equal(await page.locator('#normals tr').count(),3);assert.equal(await page.locator('#meshes tr').count(),10);assert.equal(await page.locator('#heights tr').count(),10);
 await page.screenshot({path:'build/terrain-codec-comparison/report.png',fullPage:true});
 assert.deepEqual(errors,[]);console.log(JSON.stringify({meshResults,normalResults,heightResults,precision,allChecksPassed:true}));
}finally{await browser.close();}})().catch(e=>{console.error(e);process.exitCode=1;});
