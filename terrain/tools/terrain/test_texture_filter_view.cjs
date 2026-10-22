const {chromium}=require('playwright'),assert=require('node:assert/strict');
(async()=>{const b=await chromium.launch({channel:'msedge',headless:true});try{const p=await b.newPage({viewport:{width:1440,height:1080}}),errors=[],report=[];p.on('pageerror',e=>errors.push(e.message));
const ready=()=>p.waitForFunction(()=>window.nationalMeshStatus?.tiles&&!dirty&&!selectionDirty&&!active&&!queue.length&&!landColorLoading&&!reliefLoading&&!lodTransition,null,{timeout:120000});
const state=()=>p.evaluate(()=>({epoch,serial,camera:[...center,distance,bearing,tilt],vertices:nationalMeshStatus.vertices,bytes:nationalMeshStatus.requiredBytes,shading:$('meshShading').checked,texture:$('textureType').value}));
const hash=()=>p.evaluate(()=>{draw();const a=new Uint8Array(canvas.width*canvas.height*4);gl.readPixels(0,0,canvas.width,canvas.height,gl.RGBA,gl.UNSIGNED_BYTE,a);let h=2166136261;for(let y=300;y<canvas.height*.75;y++)for(let x=canvas.width*.25|0;x<canvas.width*.75;x++){const at=(y*canvas.width+x)*4;for(let c=0;c<3;c++)h=Math.imul(h^a[at+c],16777619);}return h>>>0;});
for(const region of ['fuji','tsukuba','yari']){
 await p.goto(`http://127.0.0.1:61452/?dataset=allocation-error-detail20-direct-e800-mixed&region=${region}&level=3&curve=loop&divisions=1&smooth=1&texture=surface&meshShade=1&landColorSize=2048&priorityVertices=0&poolVertices=0&surfaceColor=vertex`);await ready();assert.equal(await p.locator('#surfaceColorMode').count(),0);assert.equal(await p.inputValue('#surfaceFilter'),'cubicAniso');
 for(const view of ['near','oblique']){
  await p.evaluate(v=>{distance=v==='near'?14000:160000;tilt=v==='near'?45:70;$('tilt').value=tilt;selectionDirty=dirty=true;},view);await ready();const before=await state(),hashes={};
  for(const mode of ['linear','cubic','aniso','cubicAniso']){
   await p.selectOption('#surfaceFilter',mode);await ready();assert.deepEqual(await state(),before);assert.equal(await p.evaluate(()=>gl.getError()),0);assert.equal(await p.evaluate(()=>nationalMeshStatus.surfaceFilter),mode);hashes[mode]=await hash();
   const milliseconds=await p.evaluate(()=>{const start=performance.now();draw();gl.finish();return performance.now()-start;});report.push({region,view,mode,milliseconds});
   if(region==='fuji')await p.screenshot({path:`build/texture-filter-${view}-${mode}.png`});
  }
  assert.notEqual(hashes.linear,hashes.cubic);if(view==='oblique')assert.notEqual(hashes.linear,hashes.aniso);
  await p.selectOption('#surfaceFilter','linear');await ready();assert.equal(await hash(),hashes.linear);console.log(region,view,hashes);
 }
 await p.selectOption('#surfaceFilter','cubicAniso');await p.selectOption('#textureType','surfaceShade');await ready();assert.equal(await p.evaluate(()=>gl.getError()),0);
 await p.selectOption('#quality','400');await p.waitForFunction(()=>manifest?.sharedAllocation?.errorDetailPolicy?.baseErrorMeters===400&&!dirty&&!active&&!queue.length&&!selectionDirty&&!landColorLoading&&!reliefLoading);assert.equal(await p.inputValue('#surfaceFilter'),'cubicAniso');assert.equal(await p.inputValue('#textureType'),'surfaceShade');assert.equal(await p.isChecked('#meshShading'),true);
}
require('node:fs').writeFileSync('build/texture-filter-view.json',JSON.stringify(report,null,2));assert.deepEqual(errors,[]);}finally{await b.close()}})().catch(e=>{console.error(e);process.exit(1)});
