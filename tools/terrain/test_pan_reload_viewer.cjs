// 実際のドラッグで範囲外の地形を読み、応答待ち中に少し動いても表示を更新する。
const assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{const browser=await chromium.launch({headless:true,channel:'msedge'});try{
 const page=await browser.newPage({viewport:{width:1000,height:800}});const errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.addInitScript(()=>localStorage.setItem('terrain-preview-occludeLayers','false'));
 let hold=false,release,arrived;const responseGate=new Promise(resolve=>release=resolve);
 const responseArrived=new Promise(resolve=>arrived=resolve);
 await page.route('**/sqlite/mesh?**',async route=>{if(hold){hold=false;const response=await route.fetch({timeout:120000});arrived();await responseGate;await route.fulfill({response});}else await route.continue();});
 await page.goto('http://127.0.0.1:61434/terrain?lat=36.32155&lon=137.6476&size=30&fullscreen=1&levelMode=auto',{timeout:120000});
 await page.waitForFunction(()=>typeof lodBusy!=='undefined'&&!lodBusy&&!!data.overlays&&boundsContain(loadedBounds,viewFootprint().bounds),null,{timeout:120000});
 const original=await page.evaluate(()=>data.coverage.longitude);
 await page.evaluate(()=>{window.appliedCenters=[];window.terrainRequests=[];const load=terrainClient.load;terrainClient.load=url=>{const query=new URL(url,location.href).searchParams;if(query.get('stage')!=='full')window.terrainRequests.push(Number(query.get('lon')));return load(url);};const swap=swapTerrain;swapTerrain=incoming=>{window.appliedCenters.push(incoming.coverage.longitude);swap(incoming);};});
 hold=true;
 for(let i=0;i<4;i++){await page.mouse.move(700,450);await page.mouse.down();await page.mouse.move(100,450,{steps:8});await page.mouse.up();}
 await Promise.race([responseArrived,new Promise((_,reject)=>{const timer=setTimeout(()=>reject(Error('タイルの再取得がありません')),30000);timer.unref();})]);const expected=await page.evaluate(()=>window.terrainRequests.at(-1));
 await page.mouse.move(500,450);await page.mouse.down();await page.mouse.move(490,450,{steps:2});await page.mouse.up();
 assert.ok(await page.evaluate(()=>lodBusy));release();
 await page.waitForFunction(lon=>window.appliedCenters.some(v=>Math.abs(v-lon)<1e-6)&&!lodBusy,expected,{timeout:120000});
 const result=await page.evaluate(()=>({longitude:data.coverage.longitude,center:[...viewCenter],inside:boundsContain(loadedBounds,viewFootprint().bounds),applied:window.appliedCenters,error:gl.getError()}));
 assert.ok(Math.abs(result.longitude-original)>.1);assert.equal(result.inside,true);assert.equal(result.error,0);assert.deepEqual(errors,[]);
 await page.screenshot({path:'build/terrain-national/pan-reloaded.png'});
 console.log(JSON.stringify(result));
}finally{await browser.close();}})().catch(e=>{console.error(e);process.exitCode=1;});
