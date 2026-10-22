const {chromium}=require('playwright'),assert=require('node:assert/strict');
const variant=process.env.QUALITY_VARIANT||'';
const errors=(process.env.QUALITY_ERRORS||'25,50,100,200,400,800').split(',').map(Number),modeFor=e=>'error-detail20'+(variant?'-'+variant:'')+(e===25?'':'-e'+e)+'-mixed';
(async()=>{const browser=await chromium.launch({channel:'msedge',headless:true});try{const page=await browser.newPage({viewport:{width:1400,height:1000}}),failures=[];page.on('pageerror',e=>failures.push(e.message));
for(const region of ['fuji','tsukuba','yari']){
 await page.goto(`http://127.0.0.1:61452/?dataset=allocation-${modeFor(errors[0])}&region=${region}&level=3&curve=loop&divisions=2&smooth=1&contrast=2&loopRetention=0.65&priorityVertices=0&poolVertices=0`);
 await page.waitForFunction(()=>window.nationalMeshStatus?.tiles&&!selectionDirty&&!dirty&&!active&&!queue.length,null,{timeout:180000});
 const original=await page.evaluate(()=>({camera:[...center,distance,bearing,tilt],settings:[$('divisions').value,$('smooth').checked,$('loopRetention').value,$('shadeContrast').value]}));let previous=Infinity;
 for(const error of errors){
  await page.selectOption('#quality',String(error));await page.waitForURL(url=>url.searchParams.get('dataset')==='allocation-'+modeFor(error));
  await page.waitForFunction(e=>window.nationalMeshStatus?.tiles&&manifest.sharedAllocation.errorDetailPolicy.baseErrorMeters===e&&!selectionDirty&&!dirty&&!active&&!queue.length,error,{timeout:180000});
  const result=await page.evaluate(()=>({quality:$('quality').value,minimum:manifest.sharedAllocation.errorDetailPolicy.minimumAdditionalHeightErrorMeters,pruning:manifest.sharedAllocation.errorDetailPolicy.pruningEnabled,variant:qualityDescriptor(manifest.sharedAllocation.mode).variant,options:[...$('quality').options].map(o=>o.value),camera:[...center,distance,bearing,tilt],settings:[$('divisions').value,$('smooth').checked,$('loopRetention').value,$('shadeContrast').value],source:manifest.sharedAllocation.sourceVertices,regionSource:manifest.sharedAllocation.meshQuality.regions[new URLSearchParams(location.search).get('region')].sourceVertices,gl:gl.getError(),failed:failures.size,tiles:nationalMeshStatus.tiles}));
  assert.equal(result.quality,String(error));if(variant==='cutoff'){assert.equal(result.variant,'cutoff');assert.equal(result.minimum,Math.max(10,error/4));}if(variant==='direct'){assert.equal(result.variant,'direct');assert.equal(result.minimum,0);assert.equal(result.pruning,false);for(const e of errors)assert.ok(result.options.includes(String(e)));}assert.equal(result.gl,0);assert.equal(result.failed,0);assert.deepEqual(result.settings,original.settings);result.camera.forEach((v,i)=>assert.ok(Math.abs(v-original.camera[i])<.02,JSON.stringify(result)));assert.ok(result.source<previous);previous=result.source;console.log(region,error,result.regionSource,result.tiles);
  if(error===12.5||error===25||error===800)await page.screenshot({path:`build/quality-${variant||'original'}-e${error}-${region}.png`});
 }
}
assert.deepEqual(failures,[]);}finally{await browser.close()}})().catch(e=>{console.error(e);process.exit(1)});
