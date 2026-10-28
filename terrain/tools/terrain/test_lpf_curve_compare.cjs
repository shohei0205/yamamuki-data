'use strict';
const fs=require('node:fs'),path=require('node:path'),assert=require('node:assert/strict'),{chromium}=require('playwright');
(async()=>{
 const base=process.argv[3]||'http://127.0.0.1:61464';
 const out=path.resolve(process.argv[2]);fs.mkdirSync(out,{recursive:true});
 const browser=await chromium.launch({channel:'chrome',headless:true,args:['--use-angle=swiftshader','--enable-unsafe-swiftshader']});
 const page=await browser.newPage({viewport:{width:1500,height:1000}}),errors=[],rows=[];
 page.on('pageerror',e=>errors.push(e.message));
 async function ready(region,z){await page.waitForFunction(([r,z])=>window.lpfCurveStatus?.loaded&&lpfCurveStatus.region===r&&lpfCurveStatus.z===z,[region,z],{timeout:120000});const s=await page.evaluate(()=>lpfCurveStatus);assert.equal(s.error,0);return s;}
 try{
  for(const region of ['fuji','yari','tsukuba']){
   await page.goto(`${base}/?region=${region}&z=6`);rows.push(await ready(region,6));
   for(const method of ['original','filtered','curve']){await page.selectOption('#method',method);assert.equal((await ready(region,6)).method,method);}
   await page.screenshot({path:path.join(out,`${region}-z6.png`)});
   await page.selectOption('#level','8');rows.push(await ready(region,8));
   await page.mouse.move(800,650);await page.mouse.down({button:'right'});await page.mouse.move(830,615,{steps:5});await page.mouse.up({button:'right'});assert.equal((await ready(region,8)).error,0);
   await page.selectOption('#level','14');rows.push(await ready(region,14));
   await page.selectOption('#renderer','triangles');
   await page.locator('#normals').uncheck();assert.equal((await ready(region,14)).error,0);
   await page.locator('#normals').check();await page.selectOption('#renderer','ray');assert.equal((await ready(region,14)).renderer,'ray');assert.equal((await ready(region,14)).surfaceTriangles,0);await page.mouse.move(1200,600);await page.mouse.wheel(0,-750);
   await page.screenshot({path:path.join(out,`${region}-z14-close.png`)});

  }
  const signature=()=>page.evaluate(async()=>{draw();const c=document.getElementById('terrain'),g=c.getContext('webgl2'),pixels=new Uint8Array(c.width*c.height*4);g.readPixels(0,0,c.width,c.height,g.RGBA,g.UNSIGNED_BYTE,pixels);return Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',pixels))).map(v=>v.toString(16).padStart(2,'0')).join('');});
  const unchanged=await signature();
  await page.locator('#divisions').evaluate(e=>{e.value='1';e.dispatchEvent(new Event('change'));});
  await page.locator('#normals').evaluate(e=>{e.checked=false;e.dispatchEvent(new Event('change'));});
  assert.equal(await signature(),unchanged,'直接交差が近似描画の設定に依存しています');
  assert(await page.locator('#divisions').isDisabled());assert(await page.locator('#normals').isDisabled());
  assert.deepEqual(errors,[]);fs.writeFileSync(path.join(out,'verification.json'),JSON.stringify({passed:true,rows,errors},null,2));console.log(JSON.stringify({passed:true,cases:rows.length,errors}));
 }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
