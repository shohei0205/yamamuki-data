const {chromium}=require('playwright'),assert=require('node:assert/strict');
(async()=>{const browser=await chromium.launch({channel:'msedge',headless:true});try{const page=await browser.newPage({viewport:{width:1440,height:1080}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
const ready=()=>page.waitForFunction(()=>window.nationalMeshStatus?.tiles&&!selectionDirty&&!dirty&&!active&&!queue.length&&!landColorLoading&&!reliefLoading,null,{timeout:120000});
const pixels=()=>page.evaluate(()=>{draw();const p=new Uint8Array(canvas.width*canvas.height*4);gl.readPixels(0,0,canvas.width,canvas.height,gl.RGBA,gl.UNSIGNED_BYTE,p);let hash=2166136261;for(let y=canvas.height*.2|0;y<canvas.height*.65;y++)for(let x=canvas.width*.2|0;x<canvas.width*.8;x++){const at=(y*canvas.width+x)*4;for(let c=0;c<3;c++)hash=Math.imul(hash^p[at+c],16777619);}return hash>>>0;});
for(const region of ['fuji','tsukuba','yari']){
 await page.goto(`http://127.0.0.1:61452/?dataset=allocation-error-detail20-direct-e800-mixed&region=${region}&level=3&overview=1&curve=loop&divisions=2&smooth=1&contrast=2&priorityVertices=0&poolVertices=0&texture=none&meshShade=1&landColorSize=1024&reliefSize=1024`);await ready();
 const before=await page.evaluate(()=>({epoch,serial,camera:[...center,distance,bearing,tilt],vertices:nationalMeshStatus.vertices,settings:[$('divisions').value,$('smooth').checked,$('shadeContrast').value]}));const hashes={};let geometryBytes;
 assert.match(await page.locator('#textureHelp').textContent(),/焼き付け陰影/);assert.deepEqual(await page.locator('#textureType option').allTextContents(),['なし','地表','地表＋陰影']);assert.equal(await page.locator('#landColor').count(),0);assert.equal(await page.locator('#reliefMode').count(),0);
 for(const type of ['none','surface','surfaceShade']){
  await page.selectOption('#textureType',type);await ready();
  for(const mesh of [false,true]){
   await page.locator('#meshShading').setChecked(mesh);await ready();
   const r=await page.evaluate(()=>({epoch,serial,camera:[...center,distance,bearing,tilt],vertices:nationalMeshStatus.vertices,settings:[$('divisions').value,$('smooth').checked,$('shadeContrast').value],textureType:nationalMeshStatus.textureType,meshShading:nationalMeshStatus.meshShading,color:nationalMeshStatus.landColorBytes,relief:nationalMeshStatus.reliefBytes,required:nationalMeshStatus.requiredBytes,gl:gl.getError(),failed:nationalMeshStatus.failures,colorError:landColorFailure,reliefError:reliefFailure}));
   for(const k of ['epoch','serial','vertices'])assert.equal(r[k],before[k]);assert.deepEqual(r.camera,before.camera);assert.deepEqual(r.settings,before.settings);assert.equal(r.textureType,type);assert.equal(r.meshShading,mesh);assert.equal(r.gl,0);assert.equal(r.failed,0);assert.equal(r.colorError,'');assert.equal(r.reliefError,'');
   assert.equal(r.color>0,type!=='none');assert.equal(r.relief>0,type==='surfaceShade');if(type==='none')geometryBytes=r.required;assert.equal(r.required,geometryBytes+r.color+r.relief);hashes[type+'/'+mesh]=await pixels();
   assert.equal(await page.locator('#smooth').isEnabled(),mesh);assert.equal(await page.locator('#shadeContrast').isEnabled(),mesh);console.log(region,type,mesh,r.required);
  }
  assert.notEqual(hashes[type+'/false'],hashes[type+'/true']);
 }
 assert.notEqual(hashes['none/false'],hashes['surface/false']);assert.notEqual(hashes['surface/false'],hashes['surfaceShade/false']);
 // 焼き付けの強さ0は地表色だけと一致する。メッシュ陰影の状態は変えない。
 await page.locator('#meshShading').uncheck();await ready();await page.locator('#reliefStrength').evaluate(e=>{e.value='0';e.dispatchEvent(new Event('input'));});await ready();assert.equal(await pixels(),hashes['surface/false']);
 for(const size of ['512','2048','4096']){await page.selectOption('#landColorSize',size);await page.selectOption('#reliefSize',size);await ready();assert.equal(await page.evaluate(()=>gl.getError()),0);}
 await page.selectOption('#landColorSize','2048');await page.selectOption('#reliefSize','2048');await page.locator('#reliefStrength').evaluate(e=>{e.value='1';e.dispatchEvent(new Event('input'));});await ready();await page.screenshot({path:`build/texture-shading-controls-${region}.png`});
 await page.selectOption('#quality','400');await page.waitForFunction(()=>manifest?.sharedAllocation?.errorDetailPolicy?.baseErrorMeters===400&&!dirty&&!active&&!queue.length&&!selectionDirty&&!landColorLoading&&!reliefLoading);assert.equal(await page.inputValue('#textureType'),'surfaceShade');assert.equal(await page.isChecked('#meshShading'),false);assert.equal(await page.inputValue('#landColorSize'),'2048');assert.equal(await page.inputValue('#reliefSize'),'2048');assert.equal(await page.inputValue('#shadeContrast'),'2');
}
for(const [query,type,mesh] of [['landColor=coverUnlit&relief=mesh','surface',false],['landColor=cover&relief=mesh','surface',true],['landColor=cover&relief=baked','surfaceShade',false]]){await page.goto('http://127.0.0.1:61452/?dataset=allocation-error-detail20-direct-e800-mixed&region=tsukuba&priorityVertices=0&poolVertices=0&'+query);await ready();assert.equal(await page.inputValue('#textureType'),type);assert.equal(await page.isChecked('#meshShading'),mesh);}
assert.deepEqual(errors,[]);}finally{await browser.close()}})().catch(e=>{console.error(e);process.exit(1)});
