// 未取得の地域にも全国の粗い地形を出し、詳細が届くまで空白にしない。
const assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{const browser=await chromium.launch({headless:true,channel:'msedge'});let release;
 try{
  const page=await browser.newPage({viewport:{width:1000,height:800}}),errors=[],urls=[];page.on('pageerror',e=>errors.push(e.message));page.on('request',r=>urls.push(r.url()));
  await page.addInitScript(()=>window.addEventListener('message',event=>{
   if(event.data?.type==='terrain-ready'&&typeof data!=='undefined'&&data.coverage.overviewFallback){
    let low=Infinity,high=-Infinity;for(const p of data.mesh)if(Math.abs(p[0]-viewCenter[0])<30000&&Math.abs(p[1]-viewCenter[1])<30000){low=Math.min(low,p[2]);high=Math.max(high,p[2]);}
    window.overviewInitial={level:data.coverage.zoom,count:data.coverage.savedTiles,bytes:data.transferSize.compressed,heightRange:high-low,normalMaps:normalMapCount,triangles:data.indices.length/3,buffers:[overviewTerrain.vertices,overviewTerrain.indices,overviewTerrain.normals].every(b=>gl.isBuffer(b)),error:gl.getError()};
   }
  }));
  let hold=true,arrived;let gate=new Promise(resolve=>release=resolve),arrival=new Promise(resolve=>arrived=resolve);
  await page.route('**/sqlite/mesh?**',async route=>{if(hold){hold=false;const response=await route.fetch();arrived();await gate;await route.fulfill({response});}else await route.continue();});
  await page.goto('http://127.0.0.1:61434/terrain?lat=36.32155&lon=137.6476&size=30&fullscreen=1&levelMode=auto');
  await arrival;
  const coarse=await page.evaluate(()=>window.overviewInitial);
  assert.equal(coarse.level,4);assert.equal(coarse.count,4);assert.equal(coarse.normalMaps,4);assert.equal(coarse.triangles,80233);assert.equal(coarse.buffers,true);assert.equal(coarse.error,0);
  const normalDifference=await page.evaluate(()=>{
   const sample=()=>{const pixels=new Uint8Array(400*400*4);gl.readPixels(300,200,400,400,gl.RGBA,gl.UNSIGNED_BYTE,pixels);return pixels;};
   draw();const on=sample(),original=gl.uniform1i.bind(gl),count=data.indices.length;
   gl.uniform1i=(location,value)=>original(location,location===uniforms.useNormalMap?0:value);draw();const off=sample();gl.uniform1i=original;draw();
   let changed=0;for(let i=0;i<on.length;i+=4)if(Math.abs(on[i]-off[i])+Math.abs(on[i+1]-off[i+1])+Math.abs(on[i+2]-off[i+2])>3)changed++;
   return {changed,sameGeometry:count===data.indices.length};
  });
  assert.ok(normalDifference.changed>1000);assert.equal(normalDifference.sameGeometry,true);
  await page.screenshot({path:'build/terrain-national/progressive-coarse.png'});
  await page.evaluate(()=>{window.progressiveLevels=[];const swap=swapTerrain;swapTerrain=incoming=>{window.progressiveLevels.push(incoming.coverage.zoom);swap(incoming);};});
  release();
  await page.waitForFunction(()=>!!data.overlays&&!lodBusy&&!data.coverage.progressiveTarget,null,{timeout:120000});
  const levels=await page.evaluate(()=>window.progressiveLevels);assert.ok(levels.includes(8));assert.ok(levels.includes(10));assert.ok(levels.some(z=>z>10));
  const oldLongitude=await page.evaluate(()=>data.coverage.longitude);const oldLevel=await page.evaluate(()=>data.coverage.zoom);
  await page.evaluate(()=>{window.progressiveLevels=[];});
  gate=new Promise(resolve=>release=resolve);arrival=new Promise(resolve=>arrived=resolve);hold=true;
  await page.evaluate(()=>{viewCenter[0]+=200000;requestViewDraw();});await arrival;
  const pending=await page.evaluate(()=>{
   const count=overviewTerrain.count;
   const sample=()=>{const pixels=new Uint8Array(400*400*4);gl.readPixels(300,200,400,400,gl.RGBA,gl.UNSIGNED_BYTE,pixels);let terrainPixels=0;for(let i=0;i<pixels.length;i+=4)if(Math.abs(pixels[i]-247)+Math.abs(pixels[i+1]-247)+Math.abs(pixels[i+2]-242)>12)terrainPixels++;return terrainPixels;};
   overviewTerrain.count=0;draw();const without=sample();overviewTerrain.count=count;draw();const withCoarse=sample();
   return {without,withCoarse,busy:lodBusy,longitude:data.coverage.longitude,level:data.coverage.zoom,error:gl.getError()};
  });
  assert.equal(pending.busy,true);assert.equal(pending.longitude,oldLongitude);assert.equal(pending.level,oldLevel);assert.ok(pending.withCoarse>1000);assert.equal(pending.error,0);
  await page.screenshot({path:'build/terrain-national/progressive-moving.png'});release();
  await page.waitForFunction(lon=>Math.abs(data.coverage.longitude-lon)>1&&!lodBusy&&!data.coverage.progressiveTarget&&!!data.overlays,oldLongitude,{timeout:120000});
  assert.ok((await page.evaluate(()=>window.progressiveLevels)).every(level=>level>=oldLevel));
  await page.selectOption('#terrainLevel','4');await page.waitForFunction(()=>data.coverage.zoom===4&&data.coverage.levelMode==='manual'&&!lodBusy,null,{timeout:120000});
  assert.equal(urls.filter(url=>new URL(url).pathname==='/sqlite/overview').length,1);
  assert.equal(urls.some(url=>new URL(url).pathname==='/sqlite/overview'&&new URL(url).searchParams.get('level')==='8'),false);
  assert.equal(urls.some(url=>new URL(url).pathname==='/sqlite/mesh'&&new URL(url).searchParams.get('tiles').split(';').some(tile=>tile.startsWith('4,'))),false);
  assert.equal(await page.evaluate(()=>gl.getError()),0);assert.deepEqual(errors,[]);
  console.log(JSON.stringify({coarse,normalDifference,levels,pending,nationwideDownloads:1,errorCount:errors.length}));
 }finally{release?.();await browser.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
