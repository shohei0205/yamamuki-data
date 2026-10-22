const {chromium}=require('playwright'),assert=require('node:assert/strict');
(async()=>{const b=await chromium.launch({channel:'msedge',headless:true});try{
 const p=await b.newPage({viewport:{width:1440,height:1000}}),errors=[];p.on('pageerror',e=>errors.push(e.message));
 await p.goto('http://127.0.0.1:61452/?dataset=allocation-error-detail20-direct-e800-mixed&region=fuji&level=3&texture=surface&landColorSize=256&overview=1&priorityVertices=0&poolVertices=0');
 await p.waitForFunction(()=>window.nationalMeshStatus?.tiles&&!dirty&&!active&&!queue.length&&!selectionDirty&&!landColorLoading,null,{timeout:120000});
 const result=await p.evaluate(()=>{
  const read=()=>{const pixels=new Uint8Array(canvas.width*canvas.height*4);gl.readPixels(0,0,canvas.width,canvas.height,gl.RGBA,gl.UNSIGNED_BYTE,pixels);return pixels};
  const sea=p=>p[0]===77&&p[1]===123&&p[2]===146,sky=p=>p[0]===232&&p[1]===242&&p[2]===250;
  tilt=1;distance=200000;$('tilt').value=tilt;draw();const withSea=read(),saved=drawSea;drawSea=()=>{};draw();const withoutSea=read();drawSea=saved;
  let replaced=0,terrain=0,identical=0;
  for(let i=0;i<withSea.length;i+=4){const a=withSea.subarray(i,i+3),b=withoutSea.subarray(i,i+3);if(sky(b)&&sea(a))replaced++;if(!sky(b)){terrain++;if(a.every((c,k)=>c===b[k]))identical++;}}
  // 海面だけを描き、真上では全画面が海、水平近くでは上が空・下が海になることを確かめる。
  function isolated(angle){tilt=angle;gl.clearColor(.91,.95,.98,1);gl.clear(gl.COLOR_BUFFER_BIT|gl.DEPTH_BUFFER_BIT);drawSea();const pixels=read();let seaCount=0,skyCount=0;for(let i=0;i<pixels.length;i+=4){if(sea(pixels.subarray(i,i+3)))seaCount++;if(sky(pixels.subarray(i,i+3)))skyCount++;}return {seaCount,skyCount};}
  const top=isolated(1),horizon=isolated(85),error=gl.getError();tilt=45;$('tilt').value=tilt;selectionDirty=dirty=true;
  return {replaced,terrain,identical,top,horizon,error,pixels:canvas.width*canvas.height};
 });
 assert.ok(result.replaced>1000);assert.ok(result.terrain>1000);assert.ok(result.identical/result.terrain>.98);assert.equal(result.top.seaCount,result.pixels);assert.ok(result.horizon.skyCount>1000);assert.ok(result.horizon.seaCount>1000);assert.equal(result.error,0);
 await p.waitForFunction(()=>!dirty&&!active&&!queue.length&&!selectionDirty);await p.locator('details').evaluate(e=>e.open=false);await p.screenshot({path:'build/terrain-landcover-astc/sea-background.png'});assert.deepEqual(errors,[]);console.log(JSON.stringify(result));
}finally{await b.close()}})().catch(e=>{console.error(e);process.exit(1)});
